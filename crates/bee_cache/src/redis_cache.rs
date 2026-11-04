// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! A [`Cache`] backed by a Redis server.
//!
//! Written directly against `redis` rather than as an adapter over
//! `bee_kv::RedisStore`: `Cache` works in `Vec<u8>` while the KV trait works in
//! `String`, so adapting would need a byte-for-byte encoding (hex/base64),
//! would split `set(value, ttl)` into `SET` + `EXPIRE` although a single atomic
//! command exists, and would break `incr`, whose counter must stay a plain
//! numeric string. Two small wrappers beat one lossy adapter.
//!
//! With the `redis-tls` feature, a `rediss://` URL connects over TLS, and
//! `redis://user:pass@host` carries ACL credentials in the URL. Neither needs
//! a different constructor.

use std::time::Duration;

use async_trait::async_trait;
use deadpool_redis::{Runtime, Timeouts};
use redis::{ErrorKind, ServerErrorKind};

use crate::{Cache, CacheError};

/// Default upper bound on one command's round trip, so a server that accepts
/// TCP but never replies surfaces as an error instead of hanging the caller.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Construction options for [`RedisCache`].
#[derive(Debug, Clone, Copy)]
pub struct RedisOptions {
    /// Maximum pooled connections. Values below 1 are clamped to 1.
    pub max_size: usize,
    /// Upper bound on one command's round trip.
    pub response_timeout: Duration,
}

impl Default for RedisOptions {
    fn default() -> Self {
        Self { max_size: 16, response_timeout: RESPONSE_TIMEOUT }
    }
}

/// A snapshot of a [`RedisCache`] connection pool.
///
/// Counts are copied from `deadpool_redis::Status`; they are not a consistent
/// view under load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedisPoolStatus {
    /// Maximum pooled connections.
    pub max_size: usize,
    /// Connections currently open.
    pub size: usize,
    /// Idle connections ready to be handed out.
    pub available: usize,
}

/// A [`Cache`] backed by a pool of Redis connections.
///
/// # Reconnect
///
/// A checkout recycles its connection with a PING, so a connection that died
/// while parked is discarded and replaced on the next checkout — a Redis
/// restart or a dropped socket needs no action from the caller. The *command*
/// that runs into the break fails with a [`CacheError::ConnectionError`], and
/// the *next* one succeeds on a fresh connection. There is no background
/// health-check thread. Every command's reply is bounded at the configured
/// response timeout (30 seconds by default); a checkout that waits on an
/// exhausted pool has its own fixed 30-second wait. Both fail with
/// [`CacheError::ConnectionError`].
pub struct RedisCache {
    pool: deadpool_redis::Pool,
    response_timeout: Duration,
}

impl RedisCache {
    /// Create a cache by connecting to `addr` (e.g.
    /// `"redis://127.0.0.1:6379"`) with [`RedisOptions::default`].
    ///
    /// Connects eagerly: a bad address fails here rather than on the first
    /// command.
    pub async fn new(addr: &str) -> Result<Self, CacheError> {
        Self::with_options(addr, RedisOptions::default()).await
    }

    /// Create a cache with a pool of at most `max_size` connections and the
    /// default response timeout. Values below 1 are clamped to 1.
    ///
    /// Connects eagerly, like [`RedisCache::new`].
    pub async fn with_pool_size(addr: &str, max_size: usize) -> Result<Self, CacheError> {
        Self::with_options(addr, RedisOptions { max_size, ..RedisOptions::default() }).await
    }

    /// Create a cache with explicit [`RedisOptions`].
    ///
    /// Connects eagerly, like [`RedisCache::new`].
    pub async fn with_options(addr: &str, options: RedisOptions) -> Result<Self, CacheError> {
        let pool = connect_pool(addr, options.max_size, options.response_timeout).await?;
        Ok(Self { pool, response_timeout: options.response_timeout })
    }

    /// Current pool metrics: `max_size`, open `size`, idle `available`
    /// connections, and futures `waiting` for one. A snapshot, not a
    /// consistent view under load.
    pub fn status(&self) -> deadpool_redis::Status {
        self.pool.status()
    }

    /// A crate-owned snapshot of the pool: `max_size`, open `size`, and idle
    /// `available` connections.
    pub fn pool_status(&self) -> RedisPoolStatus {
        to_pool_status(self.pool.status())
    }

    /// Check out a pooled connection, bounding every reply at the configured
    /// response timeout.
    async fn checkout(&self) -> Result<deadpool_redis::Connection, CacheError> {
        let mut conn = self.pool.get().await.map_err(conn_err)?;
        conn.set_response_timeout(self.response_timeout);
        Ok(conn)
    }

    /// `DEL`, returning how many keys were removed — the primitive behind both
    /// [`Cache::delete`] and `set(.., Some(0))`, which must not fail on a
    /// missing key.
    async fn delete_count(&self, key: &str) -> Result<i64, CacheError> {
        let mut conn = self.checkout().await?;
        redis::cmd("DEL").arg(key).query_async(&mut conn).await.map_err(map_err)
    }
}

/// Build a pool for `addr` and prove it serves a command, so a bad address
/// fails here rather than on the first cache operation.
async fn connect_pool(
    addr: &str,
    max_size: usize,
    response_timeout: Duration,
) -> Result<deadpool_redis::Pool, CacheError> {
    let pool = deadpool_redis::Config::from_url(addr)
        .builder()
        .map_err(conn_err)?
        .max_size(max_size.max(1))
        .timeouts(Timeouts {
            wait: Some(Duration::from_secs(30)),
            create: Some(Duration::from_secs(5)),
            recycle: Some(Duration::from_secs(5)),
        })
        .runtime(Runtime::Tokio1)
        .build()
        .map_err(conn_err)?;
    let mut conn = pool.get().await.map_err(conn_err)?;
    conn.set_response_timeout(response_timeout);
    redis::cmd("PING").query_async::<String>(&mut conn).await.map_err(conn_err)?;
    Ok(pool)
}

#[async_trait]
impl Cache for RedisCache {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        // `Vec<u8>` is converted byte-for-byte (redis-rs special-cases it), so
        // any blob survives the trip.
        let mut conn = self.checkout().await?;
        redis::cmd("GET").arg(key).query_async::<Option<Vec<u8>>>(&mut conn).await.map_err(map_err)
    }

    async fn set(&self, key: &str, value: Vec<u8>, ttl: Option<u64>) -> Result<(), CacheError> {
        // Redis rejects `EX 0`; an already-expired entry is simply absent,
        // which is what `Some(0)` means for the in-memory cache.
        if ttl == Some(0) {
            self.delete_count(key).await?;
            return Ok(());
        }
        let mut conn = self.checkout().await?;
        set_cmd(key, &value, ttl).query_async(&mut conn).await.map_err(map_err)
    }

    async fn delete(&self, key: &str) -> Result<(), CacheError> {
        match self.delete_count(key).await? {
            0 => Err(CacheError::NotFound),
            _ => Ok(()),
        }
    }

    async fn incr(&self, key: &str) -> Result<i64, CacheError> {
        // `INCR` creates a missing key at 0 and returns the new value, which is
        // exactly the trait's contract.
        let mut conn = self.checkout().await?;
        redis::cmd("INCR").arg(key).query_async(&mut conn).await.map_err(map_err)
    }
}

/// `SET key value [EX seconds]` — one atomic command, so the value and its TTL
/// can never be split across two round trips.
pub(crate) fn set_cmd(key: &str, value: &[u8], ttl: Option<u64>) -> redis::Cmd {
    let mut cmd = redis::cmd("SET");
    cmd.arg(key).arg(value);
    if let Some(ttl) = ttl {
        cmd.arg("EX").arg(ttl);
    }
    cmd
}

/// A pool failure — a bad URL, a refused connection, an exhausted or timed-out
/// checkout — is always the connection.
pub(crate) fn conn_err(e: impl std::fmt::Display) -> CacheError {
    CacheError::ConnectionError(e.to_string())
}

/// A server-side refusal — `INCR` on a non-numeric value, `WRONGTYPE` — is a
/// complaint about the stored data, not about the connection; everything else
/// is the connection.
pub(crate) fn map_err(e: redis::RedisError) -> CacheError {
    let message = e.to_string();
    match e.kind() {
        // redis 1.x nests server error kinds: `ERR` is `Server(ResponseError)`.
        ErrorKind::Server(ServerErrorKind::ResponseError) => CacheError::SerializeError(message),
        _ => CacheError::ConnectionError(message),
    }
}

/// Copy the `deadpool` counters into the crate's own snapshot type.
pub(crate) fn to_pool_status(status: deadpool_redis::Status) -> RedisPoolStatus {
    RedisPoolStatus { max_size: status.max_size, size: status.size, available: status.available }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packed(cmd: &redis::Cmd) -> Vec<u8> {
        cmd.get_packed_command()
    }

    #[test]
    fn set_without_ttl_sends_only_key_and_value() {
        let bytes = packed(&set_cmd("k", b"v", None));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("$3\r\nSET\r\n"), "{text}");
        assert!(text.contains("$1\r\nk\r\n"), "{text}");
        assert!(text.contains("$1\r\nv\r\n"), "{text}");
        assert!(!text.contains("EX"), "{text}");
    }

    #[test]
    fn set_with_ttl_appends_ex_seconds() {
        let text = String::from_utf8_lossy(&packed(&set_cmd("k", b"v", Some(60)))).into_owned();
        assert!(text.contains("$2\r\nEX\r\n"), "{text}");
        assert!(text.contains("$2\r\n60\r\n"), "{text}");
    }

    #[test]
    fn set_is_binary_safe() {
        let value = [0xff, 0x00, 0x10];
        let bytes = packed(&set_cmd("k", &value, None));
        assert!(bytes.windows(value.len()).any(|w| w == value), "{bytes:?}");
    }

    #[test]
    fn default_options_are_sixteen_connections_and_thirty_seconds() {
        let options = RedisOptions::default();
        assert_eq!(options.max_size, 16);
        assert_eq!(options.response_timeout, RESPONSE_TIMEOUT);
    }

    #[test]
    fn pool_status_reports_the_configured_size_before_any_connection() {
        let pool = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
            .builder()
            .unwrap()
            .max_size(3)
            .runtime(Runtime::Tokio1)
            .build()
            .unwrap();
        let cache = RedisCache { pool, response_timeout: Duration::from_secs(1) };
        let status = cache.pool_status();
        assert_eq!(status.max_size, 3);
        assert_eq!(status.size, 0);
        assert_eq!(status.available, 0);
    }
}
