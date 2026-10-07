// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Env-gated e2e for the two real backends: `BEE_KV_REDIS_DSN` (e.g.
//! `redis://127.0.0.1:6379`) and `BEE_KV_MEMCACHE_ADDR` (e.g.
//! `127.0.0.1:11211`), like the other real-server suites — a backend whose
//! variable is unset is skipped.
//!
//! Both stores run one shared body through `dyn KvStore`, so the two
//! implementations are held to the same observable behavior — that parity is
//! the point of the trait.

// File-level gate: with every feature off both tests vanish, which would
// leave `exercise` (and the imports) as dead code under CI's bare
// `cargo clippy --all-targets -- -D warnings` (no features).
#![cfg(any(feature = "redis", feature = "memcached"))]

use std::time::Duration;

use bee_kv::KvStore;

/// Every semantic the trait promises, exercised once per backend.
async fn exercise(store: &dyn KvStore, tag: &str) {
    let key = |name: &str| format!("bee_kv:e2e:{tag}:{name}");

    // Hygiene: a failed earlier run can leave the counter (or another key)
    // behind, so start from a known state. Deleting a missing key is not an
    // error on either backend.
    for name in ["n", "m1", "m2", "long"] {
        store.del(&key(name)).await.unwrap();
    }

    // Round trip, present and missing.
    store.set(&key("a"), "value").await.unwrap();
    assert_eq!(store.get(&key("a")).await.unwrap().as_deref(), Some("value"));
    assert!(store.exists(&key("a")).await.unwrap());
    assert_eq!(store.get(&key("missing")).await.unwrap(), None);
    assert!(!store.exists(&key("missing")).await.unwrap());

    // Overwrite, then delete; deleting a missing key is not an error.
    store.set(&key("a"), "other").await.unwrap();
    assert_eq!(store.get(&key("a")).await.unwrap().as_deref(), Some("other"));
    store.del(&key("a")).await.unwrap();
    assert_eq!(store.get(&key("a")).await.unwrap(), None);
    store.del(&key("a")).await.unwrap();

    // Increment creates a missing key at 0; a negative amount decrements.
    assert_eq!(store.incr(&key("n"), 1).await.unwrap(), 1);
    assert_eq!(store.incr(&key("n"), 4).await.unwrap(), 5);
    assert_eq!(store.incr(&key("n"), -3).await.unwrap(), 2);

    // Batch operations preserve the requested order.
    store.mset(&[(&key("m1"), "1"), (&key("m2"), "2")]).await.unwrap();
    let values = store.mget(&[&key("m2"), &key("missing"), &key("m1")]).await.unwrap();
    assert_eq!(values, vec![Some("2".to_string()), None, Some("1".to_string())]);

    // A short TTL expires; a non-positive one expires now (deletes).
    store.set(&key("ttl"), "soon").await.unwrap();
    store.expire(&key("ttl"), 1).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(store.get(&key("ttl")).await.unwrap(), None);

    store.set(&key("ttl0"), "now").await.unwrap();
    store.expire(&key("ttl0"), 0).await.unwrap();
    assert_eq!(store.get(&key("ttl0")).await.unwrap(), None);

    // Expiring a missing key is fine on both backends.
    store.expire(&key("missing"), 10).await.unwrap();

    // A TTL past 30 days must leave the key alive (memcached reads it as an
    // absolute timestamp rather than an offset).
    store.set(&key("long"), "alive").await.unwrap();
    store.expire(&key("long"), 60 * 60 * 24 * 31).await.unwrap();
    assert_eq!(store.get(&key("long")).await.unwrap().as_deref(), Some("alive"));

    // Cleanup — the suite must leave the server as it found it.
    for name in ["n", "m1", "m2", "long"] {
        store.del(&key(name)).await.unwrap();
    }
}

#[cfg(feature = "redis")]
#[tokio::test]
async fn redis_store_satisfies_the_trait() {
    let Ok(dsn) = std::env::var("BEE_KV_REDIS_DSN") else {
        return;
    };
    if dsn.is_empty() {
        return;
    }
    let store = bee_kv::RedisStore::new(&dsn).await.unwrap();
    exercise(&store, "redis").await;
}

#[cfg(feature = "memcached")]
#[tokio::test]
async fn memcache_store_satisfies_the_trait() {
    let Ok(addr) = std::env::var("BEE_KV_MEMCACHE_ADDR") else {
        return;
    };
    if addr.is_empty() {
        return;
    }
    let store = bee_kv::MemcacheStore::new(&addr).unwrap();
    exercise(&store, "memcache").await;

    // §54 pin: memcached counters are unsigned — a decrement below zero
    // floors at 0, unlike Redis's signed negatives. Exercised only here,
    // because the shared body must stay backend-neutral.
    let floor = "bee_kv:e2e:memcache:floor";
    store.set(floor, "2").await.unwrap();
    assert_eq!(store.incr(floor, -5).await.unwrap(), 0);
    assert_eq!(store.incr(floor, -1).await.unwrap(), 0);
    store.del(floor).await.unwrap();
}
