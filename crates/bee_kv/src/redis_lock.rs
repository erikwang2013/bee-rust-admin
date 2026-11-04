// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! A distributed mutual-exclusion lock on Redis.
//!
//! [`RedisLock::try_lock`] is one `SET NX PX`, so at most one caller holds a
//! key; [`RedisLock::try_lock_with_watchdog`] additionally renews the TTL in
//! the background until the guard is released or dropped. Neither is
//! reentrant and neither blocks: a lost race returns `None` immediately.
//! [`RedisLock::acquire`] retries `try_lock` until a caller-chosen wait
//! elapses. The TTL is the safety net — a guard whose process dies is
//! reclaimed when it runs out, which is also why [`LockGuard::release`] is
//! best-effort. Release explicitly to free the key early; dropping a guard
//! does not release it.
//!
//! Every command is bounded by the configured response timeout (30 seconds by
//! default) on the same pooled checkout that [`crate::RedisStore`] uses.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::KvError;
use crate::redis_store::{RedisOptions, checkout_pool, connect_pool, op_err};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A Redis-backed lock, one key per [`RedisLock::try_lock`] call.
pub struct RedisLock {
    pool: deadpool_redis::Pool,
    response_timeout: Duration,
}

impl RedisLock {
    /// Connect to `addr` (e.g. `"redis://127.0.0.1:6379"`) with
    /// [`RedisOptions::default`].
    ///
    /// Connects eagerly, like [`crate::RedisStore::new`].
    pub async fn connect(addr: &str) -> Result<Self, KvError> {
        Self::with_options(addr, RedisOptions::default()).await
    }

    /// Connect to `addr` with a pool of at most `max_size` connections and the
    /// default response timeout. Values below 1 are clamped to 1.
    pub async fn with_pool_size(addr: &str, max_size: usize) -> Result<Self, KvError> {
        Self::with_options(addr, RedisOptions { max_size, ..RedisOptions::default() }).await
    }

    /// Connect to `addr` with explicit [`RedisOptions`].
    ///
    /// Connects eagerly, like [`RedisLock::connect`].
    pub async fn with_options(addr: &str, options: RedisOptions) -> Result<Self, KvError> {
        let pool = connect_pool(addr, options.max_size, options.response_timeout).await?;
        Ok(Self { pool, response_timeout: options.response_timeout })
    }

    pub(crate) fn from_pool(pool: deadpool_redis::Pool, response_timeout: Duration) -> Self {
        Self { pool, response_timeout }
    }

    /// Try to take the lock at `key` for `ttl`, returning `None` when another
    /// caller holds it. Never waits, and the TTL is not renewed.
    ///
    /// An empty `key` or a `ttl` below 1 ms is rejected with
    /// [`KvError::OperationFailed`].
    pub async fn try_lock(&self, key: &str, ttl: Duration) -> Result<Option<LockGuard>, KvError> {
        self.try_acquire(key, ttl, false).await
    }

    /// Retry [`RedisLock::try_lock`] until the lock is free or `wait` elapses.
    ///
    /// Tries immediately, then at most every 50 ms, and returns `Ok(None)` once
    /// `wait` is exhausted. A `wait` of zero is a single attempt, exactly like
    /// [`RedisLock::try_lock`]. The deadline is checked before each retry, so
    /// an attempt already in flight can finish up to one command timeout
    /// (30 s by default) after it. The inputs are validated as `try_lock`
    /// does; the TTL is not renewed while waiting.
    pub async fn acquire(
        &self,
        key: &str,
        ttl: Duration,
        wait: Duration,
    ) -> Result<Option<LockGuard>, KvError> {
        const RETRY: Duration = Duration::from_millis(50);
        // `None` means the wait is too large to represent; retrying forever is
        // the closest honest meaning of such a wait.
        let deadline = tokio::time::Instant::now().checked_add(wait);
        loop {
            if let Some(guard) = self.try_lock(key, ttl).await? {
                return Ok(Some(guard));
            }
            let Some(deadline) = deadline else {
                tokio::time::sleep(RETRY).await;
                continue;
            };
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            tokio::time::sleep(remaining.min(RETRY)).await;
        }
    }

    /// Like [`RedisLock::try_lock`], but a background task renews the TTL
    /// every `ttl / 3` (at least 10 ms) until the guard is released or
    /// dropped. Renewal stops as soon as the key is gone, holds a different
    /// token, or a renewal command fails.
    ///
    /// A `ttl` below 30 ms is rejected with [`KvError::OperationFailed`]: the
    /// 10 ms renewal floor could not renew it before it expired.
    ///
    /// If the process dies, renewal dies with it and the TTL reclaims the key.
    pub async fn try_lock_with_watchdog(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Option<LockGuard>, KvError> {
        self.try_acquire(key, ttl, true).await
    }

    async fn try_acquire(
        &self,
        key: &str,
        ttl: Duration,
        watchdog: bool,
    ) -> Result<Option<LockGuard>, KvError> {
        let ttl_ms = ttl.as_millis();
        if key.is_empty() || ttl_ms == 0 {
            return Err(KvError::OperationFailed(
                "lock: key must be non-empty and ttl at least 1ms".into(),
            ));
        }
        if watchdog && ttl < Duration::from_millis(30) {
            return Err(KvError::OperationFailed("lock: watchdog requires ttl >= 30ms".into()));
        }
        let ttl_ms = i64::try_from(ttl_ms).unwrap_or(i64::MAX);
        let token = new_token();

        let mut conn = checkout_pool(&self.pool, self.response_timeout).await?;
        let acquired = redis::cmd("SET")
            .arg(key)
            .arg(&token)
            .arg("NX")
            .arg("PX")
            .arg(ttl_ms)
            .query_async::<Option<String>>(&mut conn)
            .await
            .map_err(op_err)?;
        if acquired.is_none() {
            return Ok(None);
        }

        let mut guard = LockGuard {
            pool: self.pool.clone(),
            key: key.to_owned(),
            token,
            response_timeout: self.response_timeout,
            renewal: None,
        };
        if watchdog {
            guard.renewal = Some(tokio::spawn(renew_loop(
                guard.pool.clone(),
                guard.key.clone(),
                guard.token.clone(),
                ttl,
                self.response_timeout,
            )));
        }
        Ok(Some(guard))
    }
}

/// Extend `key`'s TTL every `ttl / 3` while it still holds `token`; a lost key
/// or a failed command ends the loop for good.
async fn renew_loop(
    pool: deadpool_redis::Pool,
    key: String,
    token: String,
    ttl: Duration,
    response_timeout: Duration,
) {
    const LUA: &str = r#"
if redis.call("GET", KEYS[1]) == ARGV[1] then
    return redis.call("PEXPIRE", KEYS[1], ARGV[2])
else
    return 0
end
"#;
    let ttl_ms = i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX);
    let interval = (ttl / 3).max(Duration::from_millis(10));
    loop {
        tokio::time::sleep(interval).await;
        let Ok(mut conn) = checkout_pool(&pool, response_timeout).await else {
            return;
        };
        let renewed = redis::Script::new(LUA)
            .key(&key)
            .arg(&token)
            .arg(ttl_ms)
            .invoke_async::<i64>(&mut conn)
            .await;
        if !matches!(renewed, Ok(1)) {
            return;
        }
    }
}

/// A held lock.
///
/// Call [`LockGuard::release`] to free the key early. Dropping the guard does
/// not release it — any watchdog renewal stops, but the key stays until its
/// TTL runs out.
pub struct LockGuard {
    pool: deadpool_redis::Pool,
    key: String,
    token: String,
    response_timeout: Duration,
    renewal: Option<tokio::task::JoinHandle<()>>,
}

impl LockGuard {
    /// Delete the key only if it still holds this guard's token.
    ///
    /// Returns `false` when the TTL already expired and the lock was lost,
    /// `true` when this call freed it. Stops the watchdog first. Consumes the
    /// guard, so a lock cannot be released twice.
    pub async fn release(self) -> Result<bool, KvError> {
        const LUA: &str = r#"
if redis.call("GET", KEYS[1]) == ARGV[1] then
    return redis.call("DEL", KEYS[1])
else
    return 0
end
"#;
        if let Some(renewal) = &self.renewal {
            renewal.abort();
        }
        let mut conn = checkout_pool(&self.pool, self.response_timeout).await?;
        let deleted: i64 = redis::Script::new(LUA)
            .key(&self.key)
            .arg(&self.token)
            .invoke_async(&mut conn)
            .await
            .map_err(op_err)?;
        Ok(deleted != 0)
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Some(renewal) = &self.renewal {
            renewal.abort();
        }
    }
}

/// Unique per process and call: pid, wall-clock nanos, and a counter.
fn new_token() -> String {
    let nanos =
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_nanos());
    format!("{}-{nanos}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pool that never connects: the validation below runs before checkout.
    fn offline_lock() -> RedisLock {
        let pool = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
            .builder()
            .unwrap()
            .max_size(1)
            .runtime(deadpool_redis::Runtime::Tokio1)
            .build()
            .unwrap();
        RedisLock::from_pool(pool, RedisOptions::default().response_timeout)
    }

    #[tokio::test]
    async fn rejects_an_empty_key_or_a_sub_millisecond_ttl() {
        let lock = offline_lock();
        for (key, ttl) in [("", Duration::from_secs(1)), ("k", Duration::from_micros(999))] {
            let Err(err) = lock.try_lock(key, ttl).await else {
                panic!("expected an error for key {key:?}, ttl {ttl:?}");
            };
            assert!(matches!(err, KvError::OperationFailed(_)), "{err:?}");
        }
    }

    #[tokio::test]
    async fn watchdog_rejects_a_ttl_below_thirty_milliseconds_before_checkout() {
        let lock = offline_lock();
        for ttl in [Duration::from_millis(10), Duration::from_millis(29)] {
            let Err(err) = lock.try_lock_with_watchdog("k", ttl).await else {
                panic!("expected an error for watchdog ttl {ttl:?}");
            };
            assert!(matches!(err, KvError::OperationFailed(_)), "{err:?}");
        }

        // 30 ms passes validation and reaches the (unreachable) checkout.
        let err = match lock.try_lock_with_watchdog("k", Duration::from_millis(30)).await {
            Err(err) => err,
            Ok(_) => panic!("expected a checkout failure for a 30 ms watchdog ttl"),
        };
        assert!(matches!(err, KvError::ConnectionError(_)), "{err:?}");
    }

    #[tokio::test]
    async fn acquire_validates_before_checkout_and_a_zero_wait_still_tries_once() {
        let lock = offline_lock();
        let Err(err) = lock.acquire("", Duration::from_secs(1), Duration::from_secs(1)).await
        else {
            panic!("expected a validation error");
        };
        assert!(matches!(err, KvError::OperationFailed(_)), "{err:?}");

        // `wait == 0` still makes the first attempt, which reaches the
        // unreachable checkout instead of returning `None` without connecting.
        let Err(err) = lock.acquire("k", Duration::from_secs(1), Duration::ZERO).await else {
            panic!("expected a checkout failure");
        };
        assert!(matches!(err, KvError::ConnectionError(_)), "{err:?}");
    }
}
