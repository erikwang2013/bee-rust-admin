// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use std::time::Duration;

use async_trait::async_trait;
use deadpool_redis::{Runtime, Timeouts};

use crate::{KvError, KvStore};

/// Default upper bound on one command's round trip, so a server that accepts
/// TCP but never replies surfaces as an error instead of hanging the caller.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Construction options for [`RedisStore`].
#[cfg(feature = "redis")]
#[derive(Debug, Clone, Copy)]
pub struct RedisOptions {
    /// Maximum pooled connections. Values below 1 are clamped to 1.
    pub max_size: usize,
    /// Upper bound on one command's round trip.
    pub response_timeout: Duration,
}

#[cfg(feature = "redis")]
impl Default for RedisOptions {
    fn default() -> Self {
        Self { max_size: 16, response_timeout: RESPONSE_TIMEOUT }
    }
}

/// A snapshot of a [`RedisStore`] connection pool.
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

/// A [`KvStore`] backed by a pool of Redis connections.
///
/// # Reconnect
///
/// A checkout recycles its connection with a PING, so a connection that died
/// while parked is discarded and replaced on the next checkout — a Redis
/// restart or a dropped socket needs no action from the caller. The *command*
/// that runs into the break fails with a [`KvError`], and the *next* one
/// succeeds on a fresh connection. There is no background health-check
/// thread. Every command's reply is bounded at the configured response
/// timeout (30 seconds by default); a checkout that waits on an exhausted
/// pool has its own fixed 30-second wait. A checkout failure is
/// [`KvError::ConnectionError`], while a command that hits its bound is
/// [`KvError::OperationFailed`].
#[cfg(feature = "redis")]
pub struct RedisStore {
    pool: deadpool_redis::Pool,
    response_timeout: Duration,
}

#[cfg(feature = "redis")]
impl RedisStore {
    /// Create a new `RedisStore` by connecting to the given `addr` (e.g.
    /// `"redis://127.0.0.1:6379"`) with [`RedisOptions::default`].
    ///
    /// Connects eagerly: a bad address fails here rather than on the first
    /// command.
    pub async fn new(addr: &str) -> Result<Self, KvError> {
        Self::with_options(addr, RedisOptions::default()).await
    }

    /// Create a new `RedisStore` with a pool of at most `max_size`
    /// connections and the default response timeout. Values below 1 are
    /// clamped to 1.
    ///
    /// Connects eagerly, like [`RedisStore::new`].
    pub async fn with_pool_size(addr: &str, max_size: usize) -> Result<Self, KvError> {
        Self::with_options(addr, RedisOptions { max_size, ..RedisOptions::default() }).await
    }

    /// Create a new `RedisStore` with explicit [`RedisOptions`].
    ///
    /// Connects eagerly, like [`RedisStore::new`].
    pub async fn with_options(addr: &str, options: RedisOptions) -> Result<Self, KvError> {
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

    /// A lock sharing this store's connection pool and response timeout.
    pub fn lock(&self) -> crate::RedisLock {
        crate::RedisLock::from_pool(self.pool.clone(), self.response_timeout)
    }

    /// Check out a pooled connection, bounding every reply at the configured
    /// response timeout.
    async fn checkout(&self) -> Result<deadpool_redis::Connection, KvError> {
        checkout_pool(&self.pool, self.response_timeout).await
    }
}

/// Copy the `deadpool` counters into the crate's own snapshot type.
pub(crate) fn to_pool_status(status: deadpool_redis::Status) -> RedisPoolStatus {
    RedisPoolStatus { max_size: status.max_size, size: status.size, available: status.available }
}

/// Check out a pooled connection, bounding every reply at `response_timeout`.
pub(crate) async fn checkout_pool(
    pool: &deadpool_redis::Pool,
    response_timeout: Duration,
) -> Result<deadpool_redis::Connection, KvError> {
    let mut conn = pool.get().await.map_err(conn_err)?;
    conn.set_response_timeout(response_timeout);
    Ok(conn)
}

/// Build a pool for `addr` and prove it serves a command, so a bad address
/// fails here rather than on the first store operation.
pub(crate) async fn connect_pool(
    addr: &str,
    max_size: usize,
    response_timeout: Duration,
) -> Result<deadpool_redis::Pool, KvError> {
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

#[cfg(feature = "redis")]
#[async_trait]
impl KvStore for RedisStore {
    async fn get(&self, key: &str) -> Result<Option<String>, KvError> {
        let mut conn = self.checkout().await?;
        redis::cmd("GET").arg(key).query_async(&mut conn).await.map_err(op_err)
    }

    async fn set(&self, key: &str, value: &str) -> Result<(), KvError> {
        let mut conn = self.checkout().await?;
        redis::cmd("SET").arg(key).arg(value).query_async(&mut conn).await.map_err(op_err)
    }

    async fn del(&self, key: &str) -> Result<(), KvError> {
        let mut conn = self.checkout().await?;
        redis::cmd("DEL").arg(key).query_async(&mut conn).await.map_err(op_err)
    }

    async fn exists(&self, key: &str) -> Result<bool, KvError> {
        let mut conn = self.checkout().await?;
        redis::cmd("EXISTS").arg(key).query_async(&mut conn).await.map_err(op_err)
    }

    async fn incr(&self, key: &str, amount: i64) -> Result<i64, KvError> {
        let mut conn = self.checkout().await?;
        incr_cmd(key, amount).query_async(&mut conn).await.map_err(op_err)
    }

    async fn expire(&self, key: &str, seconds: i64) -> Result<(), KvError> {
        let mut conn = self.checkout().await?;
        redis::cmd("EXPIRE").arg(key).arg(seconds).query_async(&mut conn).await.map_err(op_err)
    }

    async fn mget(&self, keys: &[&str]) -> Result<Vec<Option<String>>, KvError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.checkout().await?;
        redis::cmd("MGET").arg(keys).query_async(&mut conn).await.map_err(op_err)
    }

    async fn mset(&self, pairs: &[(&str, &str)]) -> Result<(), KvError> {
        if pairs.is_empty() {
            return Ok(());
        }
        let mut cmd = redis::cmd("MSET");
        for (k, v) in pairs {
            cmd.arg(k).arg(v);
        }
        let mut conn = self.checkout().await?;
        cmd.query_async(&mut conn).await.map_err(op_err)
    }
}

pub(crate) fn conn_err(e: impl std::fmt::Display) -> KvError {
    KvError::ConnectionError(e.to_string())
}

pub(crate) fn op_err(e: impl std::fmt::Display) -> KvError {
    KvError::OperationFailed(e.to_string())
}

/// `INCR` for 1 (Redis's atomic increment), `INCRBY` otherwise.
pub(crate) fn incr_cmd(key: &str, amount: i64) -> redis::Cmd {
    let mut cmd = redis::cmd(if amount == 1 { "INCR" } else { "INCRBY" });
    cmd.arg(key);
    if amount != 1 {
        cmd.arg(amount);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packed(cmd: &redis::Cmd) -> String {
        String::from_utf8_lossy(&cmd.get_packed_command()).into_owned()
    }

    #[test]
    fn incr_uses_incr_for_one_and_incrby_for_others() {
        let one = packed(&incr_cmd("visits", 1));
        assert!(one.contains("$4\r\nINCR\r\n"), "{one}");
        assert!(!one.contains("INCRBY"), "{one}");

        let many = packed(&incr_cmd("visits", 5));
        assert!(many.contains("$6\r\nINCRBY\r\n"), "{many}");
        assert!(many.contains("$1\r\n5\r\n"), "{many}");

        let zero = packed(&incr_cmd("visits", 0));
        assert!(zero.contains("$6\r\nINCRBY\r\n"), "{zero}");
    }

    #[test]
    fn packed_command_contains_key() {
        let cmd = packed(&incr_cmd("page/1", 1));
        assert!(cmd.contains("$6\r\npage/1\r\n"), "{cmd}");
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
        let store = RedisStore { pool, response_timeout: Duration::from_secs(1) };
        let status = store.pool_status();
        assert_eq!(status.max_size, 3);
        assert_eq!(status.size, 0);
        assert_eq!(status.available, 0);
    }
}
