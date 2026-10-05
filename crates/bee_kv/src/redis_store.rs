// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use std::time::Duration;

use async_trait::async_trait;
use redis::AsyncConnectionConfig;

use crate::{KvError, KvStore};

/// A [`KvStore`] backed by a Redis server over an async multiplexed
/// connection.
#[cfg(feature = "redis")]
pub struct RedisStore {
    conn: redis::aio::MultiplexedConnection,
}

#[cfg(feature = "redis")]
impl RedisStore {
    /// Create a new `RedisStore` by connecting to the given `addr` (e.g.
    /// `"redis://127.0.0.1:6379"`).
    pub async fn new(addr: &str) -> Result<Self, KvError> {
        let client = redis::Client::open(addr)
            .map_err(|e| KvError::ConnectionError(format!("failed to create client: {e}")))?;
        let conn = client
            .get_multiplexed_async_connection_with_config(
                &AsyncConnectionConfig::new()
                    .set_connection_timeout(Duration::from_secs(5))
                    .set_response_timeout(Duration::from_secs(30)),
            )
            .await
            .map_err(|e| KvError::ConnectionError(format!("failed to connect: {e}")))?;
        Ok(Self { conn })
    }
}

#[cfg(feature = "redis")]
#[async_trait]
impl KvStore for RedisStore {
    async fn get(&self, key: &str) -> Result<Option<String>, KvError> {
        redis::cmd("GET")
            .arg(key)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| KvError::OperationFailed(e.to_string()))
    }

    async fn set(&self, key: &str, value: &str) -> Result<(), KvError> {
        redis::cmd("SET")
            .arg(key)
            .arg(value)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| KvError::OperationFailed(e.to_string()))
    }

    async fn del(&self, key: &str) -> Result<(), KvError> {
        redis::cmd("DEL")
            .arg(key)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| KvError::OperationFailed(e.to_string()))
    }

    async fn exists(&self, key: &str) -> Result<bool, KvError> {
        redis::cmd("EXISTS")
            .arg(key)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| KvError::OperationFailed(e.to_string()))
    }

    async fn incr(&self, key: &str, amount: i64) -> Result<i64, KvError> {
        incr_cmd(key, amount)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| KvError::OperationFailed(e.to_string()))
    }

    async fn expire(&self, key: &str, seconds: i64) -> Result<(), KvError> {
        redis::cmd("EXPIRE")
            .arg(key)
            .arg(seconds)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| KvError::OperationFailed(e.to_string()))
    }

    async fn mget(&self, keys: &[&str]) -> Result<Vec<Option<String>>, KvError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        redis::cmd("MGET")
            .arg(keys)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| KvError::OperationFailed(e.to_string()))
    }

    async fn mset(&self, pairs: &[(&str, &str)]) -> Result<(), KvError> {
        if pairs.is_empty() {
            return Ok(());
        }
        let mut cmd = redis::cmd("MSET");
        for (k, v) in pairs {
            cmd.arg(k).arg(v);
        }
        cmd.query_async(&mut self.conn.clone())
            .await
            .map_err(|e| KvError::OperationFailed(e.to_string()))
    }
}

/// `INCR` for 1 (Redis's atomic increment), `INCRBY` otherwise.
fn incr_cmd(key: &str, amount: i64) -> redis::Cmd {
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
}
