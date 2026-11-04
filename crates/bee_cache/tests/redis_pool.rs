// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Env-gated e2e: a pool far smaller than the concurrent workload still
//! serves every request.
//!
//! Gated by `BEE_KV_REDIS_DSN` like the other real-server suites: without it
//! the test returns immediately. Thirty-two concurrent `GET`s share two
//! connections — callers queue on `wait` instead of failing.

#![cfg(feature = "redis")]

use std::sync::Arc;
use std::time::Duration;

use bee_cache::{Cache, RedisCache, RedisOptions};

#[tokio::test]
async fn concurrent_gets_with_a_small_pool_all_succeed() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let cache = Arc::new(RedisCache::with_pool_size(&dsn, 2).await.unwrap());
    let key = format!("bee_cache:pool:probe:{}", std::process::id());
    cache.set(&key, b"ok".to_vec(), None).await.unwrap();

    let mut set = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let cache = Arc::clone(&cache);
        let key = key.clone();
        set.spawn(async move { cache.get(&key).await });
    }
    while let Some(result) = set.join_next().await {
        assert_eq!(result.unwrap().unwrap().as_deref(), Some(b"ok".as_slice()));
    }
    cache.delete(&key).await.unwrap();
}

#[tokio::test]
async fn with_options_serves_commands_and_reports_status() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let options = RedisOptions { max_size: 1, response_timeout: Duration::from_secs(5) };
    let cache = RedisCache::with_options(&dsn, options).await.unwrap();
    let key = format!("bee_cache:pool:options:{}", std::process::id());

    cache.set(&key, b"ok".to_vec(), None).await.unwrap();
    assert_eq!(cache.get(&key).await.unwrap().as_deref(), Some(b"ok".as_slice()));
    assert_eq!(cache.status().max_size, 1);
    let pool = cache.pool_status();
    assert_eq!(pool.max_size, 1);
    assert_eq!(pool.size, 1);
    assert_eq!(pool.available, 1);
    cache.delete(&key).await.unwrap();
}
