// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Connection and command plumbing shared by the Sentinel and Cluster caches.
//!
//! A backend only decides how a connection is obtained; the command shapes
//! live here once.

use async_trait::async_trait;

use crate::CacheError;
use crate::redis_cache::{map_err, set_cmd};

/// Connection source shared by the Sentinel and Cluster caches. The default
/// methods hold the command shapes, so a backend only decides how a connection
/// is obtained.
#[async_trait]
pub(crate) trait HaCacheBackend: Send + Sync {
    type Conn: redis::aio::ConnectionLike + Send;

    async fn checkout(&self) -> Result<Self::Conn, CacheError>;

    fn status(&self) -> deadpool_redis::Status;

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        // `Vec<u8>` is converted byte-for-byte (redis-rs special-cases it), so
        // any blob survives the trip.
        let mut conn = self.checkout().await?;
        redis::cmd("GET").arg(key).query_async::<Option<Vec<u8>>>(&mut conn).await.map_err(map_err)
    }

    /// `DEL`, returning how many keys were removed — the primitive behind both
    /// [`Cache::delete`](crate::Cache::delete) and `set(.., Some(0))`, which
    /// must not fail on a missing key.
    async fn delete_count(&self, key: &str) -> Result<i64, CacheError> {
        let mut conn = self.checkout().await?;
        redis::cmd("DEL").arg(key).query_async(&mut conn).await.map_err(map_err)
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

/// Forward every [`Cache`](crate::Cache) method to the type's `backend` field.
macro_rules! impl_cache_for_ha {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl $crate::Cache for $ty {
            async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, $crate::CacheError> {
                self.backend.get(key).await
            }

            async fn set(
                &self,
                key: &str,
                value: Vec<u8>,
                ttl: Option<u64>,
            ) -> Result<(), $crate::CacheError> {
                self.backend.set(key, value, ttl).await
            }

            async fn delete(&self, key: &str) -> Result<(), $crate::CacheError> {
                self.backend.delete(key).await
            }

            async fn incr(&self, key: &str) -> Result<i64, $crate::CacheError> {
                self.backend.incr(key).await
            }
        }
    };
}
pub(crate) use impl_cache_for_ha;
