// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! A [`Cache`] backed by a Redis server.
//!
//! Written directly against `redis` rather than as an adapter over
//! `bee_kv::RedisStore`: `Cache` works in `Vec<u8>` while the KV trait works in
//! `String`, so adapting would need a byte-for-byte encoding (hex/base64),
//! would split `set(value, ttl)` into `SET` + `EXPIRE` although a single atomic
//! command exists, and would break `incr`, whose counter must stay a plain
//! numeric string. Two small wrappers beat one lossy adapter.

use std::time::Duration;

use async_trait::async_trait;
use redis::ErrorKind;
use redis::aio::{ConnectionManager, ConnectionManagerConfig};

use crate::{Cache, CacheError};

/// A [`Cache`] backed by a Redis server over a reconnecting async connection
/// manager.
///
/// # Reconnect
///
/// The connection is recovered automatically — a Redis restart or a dropped
/// socket needs no action from the caller — but the *command* that runs into
/// the broken connection fails with a [`CacheError::ConnectionError`], and the
/// *next* one succeeds on a fresh connection. There is no background
/// health-check thread: the manager only reconnects when a command finds the
/// connection dead (with exponential backoff and jitter between attempts), so
/// a caller that must not surface the break has to retry.
pub struct RedisCache {
    conn: ConnectionManager,
}

impl RedisCache {
    /// Create a cache by connecting to `addr` (e.g.
    /// `"redis://127.0.0.1:6379"`).
    ///
    /// Connects eagerly: a bad address fails here rather than on the first
    /// command.
    pub async fn new(addr: &str) -> Result<Self, CacheError> {
        let client = redis::Client::open(addr)
            .map_err(|e| CacheError::ConnectionError(format!("failed to create client: {e}")))?;
        let conn = ConnectionManager::new_with_config(
            client,
            ConnectionManagerConfig::new()
                .set_connection_timeout(Duration::from_secs(5))
                .set_response_timeout(Duration::from_secs(30)),
        )
        .await
        .map_err(|e| CacheError::ConnectionError(format!("failed to connect: {e}")))?;
        Ok(Self { conn })
    }

    /// `DEL`, returning how many keys were removed — the primitive behind both
    /// [`Cache::delete`] and `set(.., Some(0))`, which must not fail on a
    /// missing key.
    async fn delete_count(&self, key: &str) -> Result<i64, CacheError> {
        redis::cmd("DEL").arg(key).query_async(&mut self.conn.clone()).await.map_err(map_err)
    }
}

#[async_trait]
impl Cache for RedisCache {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        // `Vec<u8>` is converted byte-for-byte (redis-rs special-cases it), so
        // any blob survives the trip.
        redis::cmd("GET")
            .arg(key)
            .query_async::<Option<Vec<u8>>>(&mut self.conn.clone())
            .await
            .map_err(map_err)
    }

    async fn set(&self, key: &str, value: Vec<u8>, ttl: Option<u64>) -> Result<(), CacheError> {
        // Redis rejects `EX 0`; an already-expired entry is simply absent,
        // which is what `Some(0)` means for the in-memory cache.
        if ttl == Some(0) {
            self.delete_count(key).await?;
            return Ok(());
        }
        set_cmd(key, &value, ttl).query_async(&mut self.conn.clone()).await.map_err(map_err)
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
        redis::cmd("INCR").arg(key).query_async(&mut self.conn.clone()).await.map_err(map_err)
    }
}

/// `SET key value [EX seconds]` — one atomic command, so the value and its TTL
/// can never be split across two round trips.
fn set_cmd(key: &str, value: &[u8], ttl: Option<u64>) -> redis::Cmd {
    let mut cmd = redis::cmd("SET");
    cmd.arg(key).arg(value);
    if let Some(ttl) = ttl {
        cmd.arg("EX").arg(ttl);
    }
    cmd
}

/// A server-side refusal — `INCR` on a non-numeric value, `WRONGTYPE` — is a
/// complaint about the stored data, not about the connection; everything else
/// is the connection.
fn map_err(e: redis::RedisError) -> CacheError {
    let message = e.to_string();
    match e.kind() {
        ErrorKind::ResponseError => CacheError::SerializeError(message),
        _ => CacheError::ConnectionError(message),
    }
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
}
