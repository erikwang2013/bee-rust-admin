// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! A [`KvStore`](crate::KvStore) backed by a Redis Cluster.
//!
//! Construct with one or more seed node URLs, e.g.
//! `RedisClusterStore::new(&["redis://127.0.0.1:7000"], 16)`. The cluster
//! client discovers the other nodes and follows slot moves, so a topology
//! change needs no action from the caller beyond the usual retry of the command
//! that hit it.
//!
//! [`KvStore::mget`](crate::KvStore::mget) and [`KvStore::mset`](crate::KvStore::mset)
//! are executed as sequential single-key
//! `GET`s and `SET`s, because `MGET`/`MSET` spanning hash slots fail with
//! `CROSSSLOT`. Reads and writes still route correctly: the pooled
//! `ClusterConnection` sends each key to its own node. Neither call is atomic:
//! a failing `mset` leaves the keys written before it in place, and `mget`
//! reads each key at a different instant.
//!
//! ACL credentials belong in the seed URLs (`redis://user:pass@host:7000`);
//! with the `redis-tls` feature `rediss://` selects TLS.
//!
//! # Response timeout
//!
//! A cluster command is **not** bounded by [`RedisOptions::response_timeout`]:
//! `deadpool-redis` 0.23's cluster `Config` exposes no response timeout (redis
//! 1.6's `ClusterClientBuilder` has one, but the pool never passes it), and the
//! async `ClusterConnection` exposes no setter. Only pool checkout
//! waits (30 seconds), connection creation (5 seconds), and the eager
//! construction `PING` (30 seconds) are bounded, so a node that accepts TCP but
//! never replies can hang a later command until the socket closes. A checkout
//! failure is [`KvError::ConnectionError`]; a command-level server refusal is
//! [`KvError::OperationFailed`].

use async_trait::async_trait;
use deadpool_redis::cluster::Config;
use deadpool_redis::{Runtime, Timeouts};

use crate::KvError;
use crate::redis_ha::{HaBackend, impl_kv_store_for_ha};
use crate::redis_store::{RedisOptions, RedisPoolStatus, conn_err, to_pool_status};

/// A [`KvStore`](crate::KvStore) backed by a pool of Redis Cluster
/// connections.
pub struct RedisClusterStore {
    backend: ClusterBackend,
}

struct ClusterBackend {
    pool: deadpool_redis::cluster::Pool,
}

#[async_trait]
impl HaBackend for ClusterBackend {
    type Conn = deadpool_redis::cluster::Connection;

    async fn checkout(&self) -> Result<Self::Conn, KvError> {
        self.pool.get().await.map_err(conn_err)
    }

    fn status(&self) -> deadpool_redis::Status {
        self.pool.status()
    }

    fn batched(&self) -> bool {
        false
    }
}

impl RedisClusterStore {
    /// Create a store from `urls` seed nodes (e.g.
    /// `["redis://127.0.0.1:7000"]`) with a pool of at most `max_size`
    /// connections and [`RedisOptions::default`] otherwise.
    ///
    /// Connects eagerly: an unreachable cluster fails here rather than on the
    /// first command.
    pub async fn new(urls: &[&str], max_size: usize) -> Result<Self, KvError> {
        Self::with_options(urls, RedisOptions { max_size, ..RedisOptions::default() }).await
    }

    /// Create a store with explicit [`RedisOptions`].
    ///
    /// Only `max_size` applies to cluster commands; see the module docs for why
    /// `response_timeout` is not enforced. Connects eagerly, like
    /// [`RedisClusterStore::new`].
    pub async fn with_options(urls: &[&str], options: RedisOptions) -> Result<Self, KvError> {
        let pool = connect_pool(urls, options.max_size).await?;
        Ok(Self { backend: ClusterBackend { pool } })
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

/// Build a cluster pool and prove a node serves a command, so a bad URL or an
/// unreachable cluster fails at construction.
async fn connect_pool(
    urls: &[&str],
    max_size: usize,
) -> Result<deadpool_redis::cluster::Pool, KvError> {
    let urls = urls.iter().map(|url| url.to_string()).collect::<Vec<_>>();
    let pool = Config::from_urls(urls)
        .builder()
        .map_err(conn_err)?
        .max_size(max_size.max(1))
        .timeouts(Timeouts {
            wait: Some(std::time::Duration::from_secs(30)),
            create: Some(std::time::Duration::from_secs(5)),
            recycle: Some(std::time::Duration::from_secs(5)),
        })
        .runtime(Runtime::Tokio1)
        .build()
        .map_err(conn_err)?;
    let mut conn = pool.get().await.map_err(conn_err)?;
    // Cluster commands have no configurable response bound; keep the eager
    // construction probe from hanging forever on a node that accepts TCP and
    // never replies.
    let ping = redis::cmd("PING");
    let reply = ping.query_async::<String>(&mut conn);
    tokio::time::timeout(std::time::Duration::from_secs(30), reply)
        .await
        .map_err(|_| {
            KvError::ConnectionError("cluster: PING timed out after 30s during construction".into())
        })?
        .map_err(conn_err)?;
    Ok(pool)
}

impl_kv_store_for_ha!(RedisClusterStore);

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::from_urls(vec!["redis://127.0.0.1:7000".to_string()])
    }

    #[test]
    fn config_keeps_the_seed_urls() {
        let config = config();
        assert_eq!(config.urls.as_deref(), Some(&["redis://127.0.0.1:7000".to_string()][..]));
        assert!(!config.read_from_replicas);
    }

    #[test]
    fn a_cluster_splits_multi_key_commands() {
        let pool =
            config().builder().unwrap().max_size(3).runtime(Runtime::Tokio1).build().unwrap();
        let backend = ClusterBackend { pool };
        assert!(!backend.batched());
    }

    #[test]
    fn pool_status_reports_the_configured_size_before_any_connection() {
        let pool =
            config().builder().unwrap().max_size(3).runtime(Runtime::Tokio1).build().unwrap();
        let store = RedisClusterStore { backend: ClusterBackend { pool } };
        let status = store.pool_status();
        assert_eq!(status.max_size, 3);
        assert_eq!(status.size, 0);
        assert_eq!(status.available, 0);
    }

    #[tokio::test]
    async fn an_empty_seed_list_is_a_connection_error() {
        match RedisClusterStore::new(&[], 1).await {
            Err(KvError::ConnectionError(_)) => {}
            Err(other) => panic!("expected a connection error, got {other:?}"),
            Ok(_) => panic!("an empty seed list must not build"),
        }
    }
}
