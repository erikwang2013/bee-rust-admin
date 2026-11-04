// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Connection and command plumbing shared by the Sentinel and Cluster stores.
//!
//! A backend only decides how a connection is obtained and whether multi-key
//! commands are safe; the command shapes live here once.

use async_trait::async_trait;

use crate::KvError;
use crate::redis_store::{incr_cmd, op_err};

/// Connection source shared by the Sentinel and Cluster stores. The default
/// methods hold the command shapes, so a backend only decides how a connection
/// is obtained and whether multi-key commands are safe.
#[async_trait]
pub(crate) trait HaBackend: Send + Sync {
    type Conn: redis::aio::ConnectionLike + Send;

    async fn checkout(&self) -> Result<Self::Conn, KvError>;

    fn status(&self) -> deadpool_redis::Status;

    /// `MGET`/`MSET` are safe on a single master, but not across a cluster's
    /// hash slots (`CROSSSLOT`), where they are split into single-key commands.
    fn batched(&self) -> bool;

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
        if !self.batched() {
            let mut values = Vec::with_capacity(keys.len());
            for key in keys {
                values
                    .push(redis::cmd("GET").arg(key).query_async(&mut conn).await.map_err(op_err)?);
            }
            return Ok(values);
        }
        redis::cmd("MGET").arg(keys).query_async(&mut conn).await.map_err(op_err)
    }

    async fn mset(&self, pairs: &[(&str, &str)]) -> Result<(), KvError> {
        if pairs.is_empty() {
            return Ok(());
        }
        let mut conn = self.checkout().await?;
        if !self.batched() {
            for (key, value) in pairs {
                redis::cmd("SET")
                    .arg(key)
                    .arg(value)
                    .query_async::<()>(&mut conn)
                    .await
                    .map_err(op_err)?;
            }
            return Ok(());
        }
        let mut cmd = redis::cmd("MSET");
        for (key, value) in pairs {
            cmd.arg(key).arg(value);
        }
        cmd.query_async(&mut conn).await.map_err(op_err)
    }
}

/// Forward every [`KvStore`] method to the type's `backend` field.
macro_rules! impl_kv_store_for_ha {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl $crate::KvStore for $ty {
            async fn get(&self, key: &str) -> Result<Option<String>, $crate::KvError> {
                self.backend.get(key).await
            }

            async fn set(&self, key: &str, value: &str) -> Result<(), $crate::KvError> {
                self.backend.set(key, value).await
            }

            async fn del(&self, key: &str) -> Result<(), $crate::KvError> {
                self.backend.del(key).await
            }

            async fn exists(&self, key: &str) -> Result<bool, $crate::KvError> {
                self.backend.exists(key).await
            }

            async fn incr(&self, key: &str, amount: i64) -> Result<i64, $crate::KvError> {
                self.backend.incr(key, amount).await
            }

            async fn expire(&self, key: &str, seconds: i64) -> Result<(), $crate::KvError> {
                self.backend.expire(key, seconds).await
            }

            async fn mget(&self, keys: &[&str]) -> Result<Vec<Option<String>>, $crate::KvError> {
                self.backend.mget(keys).await
            }

            async fn mset(&self, pairs: &[(&str, &str)]) -> Result<(), $crate::KvError> {
                self.backend.mset(pairs).await
            }
        }
    };
}
pub(crate) use impl_kv_store_for_ha;

#[cfg(test)]
// The Sentinel and Cluster stores share the [`HaBackend`] command methods; a
// local connection records which commands were sent, so the cluster's
// single-key `mget`/`mset` path is proven without a server.
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct MockConn {
        log: Arc<Mutex<Vec<String>>>,
    }

    /// The command name is the third `\r\n`-separated field of the packed
    /// RESP array (`*n`, bulk length, command, ...).
    fn command_name(cmd: &redis::Cmd) -> String {
        let packed = cmd.get_packed_command();
        let text = String::from_utf8_lossy(&packed);
        text.split("\r\n").nth(2).unwrap_or_default().to_string()
    }

    impl redis::aio::ConnectionLike for MockConn {
        fn req_packed_command<'a>(
            &'a mut self,
            cmd: &'a redis::Cmd,
        ) -> redis::RedisFuture<'a, redis::Value> {
            let name = command_name(cmd);
            self.log.lock().unwrap().push(name.clone());
            let value = match name.as_str() {
                "GET" => redis::Value::BulkString(b"v".to_vec()),
                "MGET" => redis::Value::Array(vec![
                    redis::Value::BulkString(b"v".to_vec()),
                    redis::Value::BulkString(b"v".to_vec()),
                ]),
                _ => redis::Value::Okay,
            };
            Box::pin(async move { Ok(value) })
        }

        fn req_packed_commands<'a>(
            &'a mut self,
            _cmd: &'a redis::Pipeline,
            _offset: usize,
            _count: usize,
        ) -> redis::RedisFuture<'a, Vec<redis::Value>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_db(&self) -> i64 {
            0
        }
    }

    struct MockBackend {
        log: Arc<Mutex<Vec<String>>>,
        batched: bool,
    }

    #[async_trait]
    impl HaBackend for MockBackend {
        type Conn = MockConn;

        async fn checkout(&self) -> Result<Self::Conn, KvError> {
            Ok(MockConn { log: Arc::clone(&self.log) })
        }

        fn status(&self) -> deadpool_redis::Status {
            deadpool_redis::Status { max_size: 1, size: 0, available: 0, waiting: 0 }
        }

        fn batched(&self) -> bool {
            self.batched
        }
    }

    fn mock(batched: bool) -> (MockBackend, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        (MockBackend { log: Arc::clone(&log), batched }, log)
    }

    #[tokio::test]
    async fn cluster_mget_and_mset_use_single_key_commands() {
        let (backend, log) = mock(false);
        let values = backend.mget(&["a", "b"]).await.unwrap();
        assert_eq!(values, vec![Some("v".to_string()), Some("v".to_string())]);
        backend.mset(&[("a", "1"), ("b", "2")]).await.unwrap();
        assert_eq!(*log.lock().unwrap(), ["GET", "GET", "SET", "SET"]);
    }

    #[tokio::test]
    async fn sentinel_mget_and_mset_use_bulk_commands() {
        let (backend, log) = mock(true);
        let values = backend.mget(&["a", "b"]).await.unwrap();
        assert_eq!(values, vec![Some("v".to_string()), Some("v".to_string())]);
        backend.mset(&[("a", "1"), ("b", "2")]).await.unwrap();
        assert_eq!(*log.lock().unwrap(), ["MGET", "MSET"]);
    }

    #[tokio::test]
    async fn empty_batches_make_no_round_trips() {
        let (backend, log) = mock(false);
        assert!(backend.mget(&[]).await.unwrap().is_empty());
        backend.mset(&[]).await.unwrap();
        assert!(log.lock().unwrap().is_empty());
    }
}
