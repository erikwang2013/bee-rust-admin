// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Env-gated e2e for the two remote backends: `BEE_KV_REDIS_DSN` (e.g.
//! `redis://127.0.0.1:6379`) and `BEE_KV_MEMCACHE_ADDR` (e.g.
//! `127.0.0.1:11211`) — a backend whose variable is unset is skipped, so the
//! suite is inert without a server.
//!
//! Both caches run one shared body through `dyn Cache`: `MemoryCache` is the
//! reference implementation of the trait, and these two must match it.

#![cfg(any(feature = "redis", feature = "memcache"))]

use std::time::Duration;

use bee_cache::{Cache, CacheError};

/// Every semantic the trait promises, exercised once per backend.
async fn exercise(cache: &dyn Cache, tag: &str) {
    let key = |name: &str| format!("bee_cache:e2e:{tag}:{name}");

    // Hygiene: a failed earlier run can leave the counter behind — reset it
    // so the first `incr` sees a missing key. Deleting a missing key is an
    // error on this trait, hence the ignored result.
    let _ = cache.delete(&key("n")).await;

    // Round trip, byte-for-byte: the payload is not valid UTF-8.
    let blob = vec![0xff, 0x00, 0xfe, 0x80, b'h', b'i'];
    cache.set(&key("blob"), blob.clone(), None).await.unwrap();
    assert_eq!(cache.get(&key("blob")).await.unwrap(), Some(blob));

    // Missing key: `get` is `None`, `delete` is an error.
    assert_eq!(cache.get(&key("missing")).await.unwrap(), None);
    assert!(matches!(cache.delete(&key("missing")).await, Err(CacheError::NotFound)));

    cache.delete(&key("blob")).await.unwrap();
    assert_eq!(cache.get(&key("blob")).await.unwrap(), None);

    // Increment creates a missing key at 0 and returns the new value.
    assert_eq!(cache.incr(&key("n")).await.unwrap(), 1);
    assert_eq!(cache.incr(&key("n")).await.unwrap(), 2);

    // Incrementing a non-numeric value is a data error, not a connection one.
    cache.set(&key("text"), b"not a number".to_vec(), None).await.unwrap();
    match cache.incr(&key("text")).await {
        Err(CacheError::SerializeError(_)) => {}
        other => panic!("expected SerializeError, got {other:?}"),
    }
    cache.delete(&key("text")).await.unwrap();

    // A short TTL expires.
    cache.set(&key("ttl"), b"soon".to_vec(), Some(1)).await.unwrap();
    assert!(cache.get(&key("ttl")).await.unwrap().is_some());
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(cache.get(&key("ttl")).await.unwrap(), None);

    // A zero TTL means already expired — absent, and not an error.
    cache.set(&key("zero"), b"gone".to_vec(), Some(0)).await.unwrap();
    assert_eq!(cache.get(&key("zero")).await.unwrap(), None);

    // A TTL past 30 days must leave the key alive (memcached reads it as an
    // absolute timestamp rather than an offset).
    cache.set(&key("long"), b"alive".to_vec(), Some(60 * 60 * 24 * 31)).await.unwrap();
    assert_eq!(cache.get(&key("long")).await.unwrap(), Some(b"alive".to_vec()));

    // Cleanup — the suite must leave the server as it found it.
    cache.delete(&key("n")).await.unwrap();
    cache.delete(&key("long")).await.unwrap();
}

#[cfg(feature = "redis")]
#[tokio::test]
async fn redis_cache_satisfies_the_trait() {
    let Ok(dsn) = std::env::var("BEE_KV_REDIS_DSN") else {
        return;
    };
    if dsn.is_empty() {
        return;
    }
    let cache = bee_cache::RedisCache::new(&dsn).await.unwrap();
    exercise(&cache, "redis").await;
}

#[cfg(feature = "memcache")]
#[tokio::test]
async fn memcache_cache_satisfies_the_trait() {
    let Ok(addr) = std::env::var("BEE_KV_MEMCACHE_ADDR") else {
        return;
    };
    if addr.is_empty() {
        return;
    }
    let cache = bee_cache::MemcacheCache::new(&addr).unwrap();
    exercise(&cache, "memcache").await;
}
