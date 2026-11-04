// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Env-gated e2e for the Redis distributed lock.
//!
//! Gated by `BEE_KV_REDIS_DSN` like the other real-server suites: without it
//! the tests return immediately.

#![cfg(feature = "redis")]

use std::time::Duration;

use bee_kv::{RedisLock, RedisStore};

fn key(name: &str) -> String {
    format!("bee_kv:lock:test:{name}:{}", std::process::id())
}

#[tokio::test]
async fn try_lock_is_exclusive_and_release_frees_it() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let lock = RedisLock::connect(&dsn).await.unwrap();
    let k = key("exclusive");

    let guard = lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().unwrap();
    assert!(lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().is_none());
    assert!(guard.release().await.unwrap());
    let guard = lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().unwrap();
    assert!(guard.release().await.unwrap());
}

#[tokio::test]
async fn ttl_expires_the_lock() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let lock = RedisLock::connect(&dsn).await.unwrap();
    let k = key("ttl");

    assert!(lock.try_lock(&k, Duration::from_millis(100)).await.unwrap().is_some());
    assert!(lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().is_none());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let guard = lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().unwrap();
    assert!(guard.release().await.unwrap());
}

#[tokio::test]
async fn acquire_waits_for_a_held_lock_and_then_succeeds() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let lock = RedisLock::connect(&dsn).await.unwrap();
    let k = key("acquire-wait");
    let held = lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().unwrap();

    // A separate connection, so the wait is real contention rather than local
    // bookkeeping.
    let waiting = {
        let dsn = dsn.clone();
        let k = k.clone();
        tokio::spawn(async move {
            let lock = RedisLock::connect(&dsn).await.unwrap();
            lock.acquire(&k, Duration::from_secs(5), Duration::from_secs(2)).await
        })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!waiting.is_finished(), "acquire returned while the lock was held");

    assert!(held.release().await.unwrap());
    let guard = waiting.await.unwrap().unwrap().expect("acquire should win after release");
    assert!(guard.release().await.unwrap());
}

#[tokio::test]
async fn acquire_returns_none_when_the_wait_elapses() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let lock = RedisLock::connect(&dsn).await.unwrap();
    let k = key("acquire-timeout");
    let held = lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().unwrap();

    let start = std::time::Instant::now();
    let result = lock.acquire(&k, Duration::from_secs(5), Duration::from_millis(150)).await;
    let elapsed = start.elapsed();
    assert!(result.unwrap().is_none(), "acquire should give up while the lock is held");
    assert!(elapsed >= Duration::from_millis(140), "returned early: {elapsed:?}");
    assert!(elapsed < Duration::from_secs(1), "wait overshot: {elapsed:?}");

    assert!(held.release().await.unwrap());
}

#[tokio::test]
async fn store_lock_shares_the_store_pool() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let store = RedisStore::new(&dsn).await.unwrap();
    let k = key("store");

    let guard = store.lock().try_lock(&k, Duration::from_secs(5)).await.unwrap().unwrap();
    assert!(guard.release().await.unwrap());
}

#[tokio::test]
async fn stale_guard_does_not_release_a_successor() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let lock = RedisLock::connect(&dsn).await.unwrap();
    let k = key("stale");

    let stale = lock.try_lock(&k, Duration::from_millis(200)).await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let live = lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().unwrap();

    assert!(!stale.release().await.unwrap());
    assert!(lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().is_none());
    assert!(live.release().await.unwrap());
}

#[tokio::test]
async fn releasing_an_expired_guard_returns_false() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let lock = RedisLock::connect(&dsn).await.unwrap();
    let k = key("expired");

    let guard = lock.try_lock(&k, Duration::from_millis(100)).await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!guard.release().await.unwrap());
}

#[tokio::test]
async fn watchdog_renews_past_the_original_ttl() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let lock = RedisLock::connect(&dsn).await.unwrap();
    let k = key("watchdog-renew");

    let guard = lock.try_lock_with_watchdog(&k, Duration::from_millis(300)).await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().is_none());
    assert!(guard.release().await.unwrap());
}

#[tokio::test]
async fn release_stops_the_watchdog() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let lock = RedisLock::connect(&dsn).await.unwrap();
    let k = key("watchdog-release");

    let guard = lock.try_lock_with_watchdog(&k, Duration::from_millis(300)).await.unwrap().unwrap();
    assert!(guard.release().await.unwrap());

    let successor = lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().is_none());
    assert!(successor.release().await.unwrap());
}

#[tokio::test]
async fn dropping_the_guard_stops_the_watchdog() {
    let Some(dsn) = std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    let lock = RedisLock::connect(&dsn).await.unwrap();
    let k = key("watchdog-drop");

    let guard = lock.try_lock_with_watchdog(&k, Duration::from_millis(300)).await.unwrap().unwrap();
    drop(guard);
    tokio::time::sleep(Duration::from_millis(800)).await;
    let guard = lock.try_lock(&k, Duration::from_secs(5)).await.unwrap().unwrap();
    assert!(guard.release().await.unwrap());
}
