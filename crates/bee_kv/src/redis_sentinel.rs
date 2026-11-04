// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! A [`KvStore`](crate::KvStore) backed by the Redis master named in Sentinel.
//!
//! Construct with the master's name and one or more Sentinel URLs, e.g.
//! `RedisSentinelStore::new("mymaster", &["redis://127.0.0.1:26379"], 16)`.
//! Sentinel resolves the master whenever the pool creates a connection, so a
//! lost connection is replaced through the Sentinels on the next checkout.
//!
//! Existing pooled connections are not role-checked: `deadpool-redis` only
//! `PING`s a connection on recycle. After a failover, a demoted master that is
//! still reachable as a replica can therefore keep serving stale connections —
//! reads may succeed and writes fail with `READONLY`. If the old master stays
//! up, rebuild the store (or drain the pool) after the failover.
//!
//! ACL credentials in the Sentinel URLs (`redis://user:pass@host:26379`)
//! authenticate the Sentinel nodes; `rediss://` (with `redis-tls`) selects TLS
//! to them. The first Sentinel URL also supplies the master connection's ACL
//! credentials, database, protocol, and TLS mode through
//! `SentinelNodeConnectionInfo`, so an ACL-protected or TLS-only master works
//! when its Sentinel URLs carry the same settings. Multi-key reads and writes
//! use `MGET`/`MSET`, which a single master serves atomically.

use std::time::Duration;

use async_trait::async_trait;
use deadpool_redis::sentinel::{Config, SentinelNodeConnectionInfo, SentinelServerType, TlsMode};
use deadpool_redis::{Runtime, Timeouts};

use crate::KvError;
use crate::redis_ha::{HaBackend, impl_kv_store_for_ha};
use crate::redis_store::{RedisOptions, RedisPoolStatus, conn_err, to_pool_status};

/// A [`KvStore`](crate::KvStore) backed by a pool of connections to the
/// current Sentinel master.
///
/// Every command's reply is bounded at the configured response timeout (30
/// seconds by default); a checkout waiting on an exhausted pool has its own
/// fixed 30-second wait. A checkout failure is [`KvError::ConnectionError`],
/// while a command that hits its bound is [`KvError::OperationFailed`].
pub struct RedisSentinelStore {
    backend: SentinelBackend,
}

struct SentinelBackend {
    pool: deadpool_redis::sentinel::Pool,
    response_timeout: Duration,
}

#[async_trait]
impl HaBackend for SentinelBackend {
    type Conn = deadpool_redis::sentinel::Connection;

    async fn checkout(&self) -> Result<Self::Conn, KvError> {
        let mut conn = self.pool.get().await.map_err(conn_err)?;
        conn.set_response_timeout(self.response_timeout);
        Ok(conn)
    }

    fn status(&self) -> deadpool_redis::Status {
        self.pool.status()
    }

    fn batched(&self) -> bool {
        true
    }
}

impl RedisSentinelStore {
    /// Create a store for `master_name`, discovering it through `sentinels`
    /// (e.g. `["redis://127.0.0.1:26379"]`) with a pool of at most `max_size`
    /// connections and [`RedisOptions::default`] otherwise.
    ///
    /// Connects eagerly: an unreachable Sentinel or master fails here rather
    /// than on the first command.
    pub async fn new(
        master_name: &str,
        sentinels: &[&str],
        max_size: usize,
    ) -> Result<Self, KvError> {
        Self::with_options(
            master_name,
            sentinels,
            RedisOptions { max_size, ..RedisOptions::default() },
        )
        .await
    }

    /// Create a store with explicit [`RedisOptions`]. `response_timeout` bounds
    /// every command on the sentinel-master connection.
    ///
    /// Connects eagerly, like [`RedisSentinelStore::new`].
    pub async fn with_options(
        master_name: &str,
        sentinels: &[&str],
        options: RedisOptions,
    ) -> Result<Self, KvError> {
        let pool = connect_pool(master_name, sentinels, options.max_size, options.response_timeout)
            .await?;
        Ok(Self { backend: SentinelBackend { pool, response_timeout: options.response_timeout } })
    }

    /// Current pool metrics: `max_size`, open `size`, idle `available`
    /// connections, and futures `waiting` for one. A snapshot, not a
    /// consistent view under load.
    pub fn status(&self) -> deadpool_redis::Status {
        self.backend.status()
    }

    /// A crate-owned snapshot of the pool: `max_size`, open `size`, and idle
    /// `available` connections.
    pub fn pool_status(&self) -> RedisPoolStatus {
        to_pool_status(self.status())
    }
}

/// Build a Sentinel-backed pool and prove the master serves a command, so a
/// bad URL or an unreachable master fails at construction.
async fn connect_pool(
    master_name: &str,
    sentinels: &[&str],
    max_size: usize,
    response_timeout: Duration,
) -> Result<deadpool_redis::sentinel::Pool, KvError> {
    let urls = sentinels.iter().map(|url| url.to_string()).collect::<Vec<_>>();
    let pool = Config::from_urls(urls, master_name.to_string(), SentinelServerType::Master)
        .with_node_connection_info(master_node_info(sentinels))
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

/// Master connection settings lifted from the first Sentinel URL. `deadpool-redis`
/// passes ACL credentials, database, protocol, and TLS to the resolved master
/// only through `SentinelNodeConnectionInfo`.
fn master_node_info(sentinels: &[&str]) -> Option<SentinelNodeConnectionInfo> {
    let info = sentinels.first()?.parse::<redis::ConnectionInfo>().ok()?;
    let tls_mode = match info.addr() {
        redis::ConnectionAddr::TcpTls { insecure, .. } => {
            Some(if *insecure { TlsMode::Insecure } else { TlsMode::Secure })
        }
        _ => None,
    };
    Some(SentinelNodeConnectionInfo {
        tls_mode,
        redis_connection_info: Some(info.redis_settings().clone().into()),
    })
}

impl_kv_store_for_ha!(RedisSentinelStore);

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::from_urls(
            vec!["redis://127.0.0.1:26379".to_string()],
            "mymaster".to_string(),
            SentinelServerType::Master,
        )
    }

    #[test]
    fn config_keeps_the_master_name_and_sentinel_urls() {
        let config = config();
        assert_eq!(config.master_name, "mymaster");
        assert_eq!(config.urls.as_deref(), Some(&["redis://127.0.0.1:26379".to_string()][..]));
    }

    #[test]
    fn the_first_sentinel_url_supplies_the_master_settings() {
        let info = master_node_info(&["rediss://user:pass@127.0.0.1:26379/2"])
            .expect("a Sentinel URL yields master connection info");
        assert!(matches!(info.tls_mode, Some(TlsMode::Secure)));
        let redis = info.redis_connection_info.expect("redis settings");
        assert_eq!(redis.username.as_deref(), Some("user"));
        assert_eq!(redis.password.as_deref(), Some("pass"));
        assert_eq!(redis.db, 2);
    }

    #[test]
    fn a_plain_sentinel_url_keeps_the_master_plaintext() {
        let info = master_node_info(&["redis://127.0.0.1:26379"]).expect("connection info");
        assert!(info.tls_mode.is_none());
        assert!(master_node_info(&[]).is_none());
    }

    #[test]
    fn a_single_master_uses_bulk_multi_key_commands() {
        let pool =
            config().builder().unwrap().max_size(3).runtime(Runtime::Tokio1).build().unwrap();
        let backend = SentinelBackend { pool, response_timeout: std::time::Duration::from_secs(1) };
        assert!(backend.batched());
    }

    #[test]
    fn pool_status_reports_the_configured_size_before_any_connection() {
        let pool =
            config().builder().unwrap().max_size(3).runtime(Runtime::Tokio1).build().unwrap();
        let store = RedisSentinelStore {
            backend: SentinelBackend { pool, response_timeout: Duration::from_secs(1) },
        };
        let status = store.pool_status();
        assert_eq!(status.max_size, 3);
        assert_eq!(status.size, 0);
        assert_eq!(status.available, 0);
    }

    #[tokio::test]
    async fn an_empty_sentinel_list_is_a_connection_error() {
        match RedisSentinelStore::new("mymaster", &[], 1).await {
            Err(KvError::ConnectionError(_)) => {}
            Err(other) => panic!("expected a connection error, got {other:?}"),
            Ok(_) => panic!("an empty sentinel list must not build"),
        }
    }
}
