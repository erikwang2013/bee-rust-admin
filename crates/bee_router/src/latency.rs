// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Opt-in 10 ms latency budget for `GET`/`HEAD` responses, with stale-cache
//! fallback.
//!
//! Enable it on a [`Router`](crate::Router) with
//! [`Router::latency`](crate::Router::latency). Write methods keep the
//! router's existing 30-second timeout: timing out a write can leave it
//! committed while the client sees an error, so they are deliberately out of
//! scope.
//!
//! The deadline bounds the **response**, not the work. Dropping a SQLite
//! `spawn_blocking` query cannot stop it, and Postgres/MySQL/Redis commands
//! continue server-side. To let an always-slow endpoint warm its cache, a
//! timed-out GET handler is moved to a bounded background task (up to
//! [`LatencyConfig::background_limit`] at once); its response is cached if it
//! is eligible, and later requests can be served stale by the fallback.
//!
//! Cache entries are only written for `GET` responses with
//! `Cache-Control: public`, no `Set-Cookie`, no `Vary`, and an exact body size
//! no larger than [`LatencyConfig::max_cache_body`]. Requests with
//! `Authorization`, `Range`, `Cache-Control: no-store`, or `no-cache` bypass
//! the cache entirely. A fallback response carries `X-Bee-Cache: stale` and
//! an `Age` header; a miss returns `504` with `X-Bee-Deadline: exceeded`.

mod cache_entry;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::body::{Body, HttpBody, to_bytes};
use axum::extract::{Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use bee_cache::Cache;
use tokio::sync::Semaphore;
use tokio::time::timeout;

/// Time held back from the handler for the cache lookup, decode, and response
/// construction. The default budget below plus this reserve stays under the
/// 10 ms SLO.
const FALLBACK_RESERVE: Duration = Duration::from_millis(2);
const DEFAULT_DEADLINE: Duration = Duration::from_millis(6);
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(60);
const DEFAULT_MAX_CACHE_BODY: usize = 64 * 1024;
const DEFAULT_BACKGROUND_LIMIT: usize = 128;
const BUCKET_US: u64 = 100;
const BUCKETS: usize = 128;

/// Configuration for [`Router::latency`](crate::Router::latency).
///
/// The default total budget is 6 ms: 4 ms for the handler and 2 ms reserved
/// for cache fallback and response construction, leaving headroom under the
/// 10 ms P99 target even when the runtime timer fires late.
#[derive(Clone)]
pub struct LatencyConfig {
    cache: Arc<dyn Cache>,
    deadline: Duration,
    fallback_reserve: Duration,
    cache_ttl: Duration,
    max_cache_body: usize,
    background: Arc<Semaphore>,
    stats: LatencyStats,
}

impl LatencyConfig {
    /// Enable the budget with `cache` as the stale-response store.
    ///
    /// Typically use an in-process [`bee_cache::MemoryCache`]: the fallback
    /// lookup only gets the 2 ms reserve, so a remote cache may miss that
    /// window. TTL is 60 seconds and the cacheable body limit is 64 KiB.
    pub fn new(cache: Arc<dyn Cache>) -> Self {
        Self {
            cache,
            deadline: DEFAULT_DEADLINE,
            fallback_reserve: FALLBACK_RESERVE,
            cache_ttl: DEFAULT_CACHE_TTL,
            max_cache_body: DEFAULT_MAX_CACHE_BODY,
            background: Arc::new(Semaphore::new(DEFAULT_BACKGROUND_LIMIT)),
            stats: LatencyStats::default(),
        }
    }

    /// Total response budget. The handler receives this value minus the fixed
    /// 2 ms fallback reserve. Panics when `deadline` is not greater than 2 ms.
    pub fn deadline(mut self, deadline: Duration) -> Self {
        assert!(
            deadline > self.fallback_reserve,
            "latency deadline must be greater than the fallback reserve"
        );
        self.deadline = deadline;
        self
    }

    /// Time reserved after the handler deadline for the cache lookup, decode,
    /// and response construction. Defaults to 2 ms. Must be smaller than the
    /// total [`deadline`](Self::deadline).
    pub fn fallback_reserve(mut self, reserve: Duration) -> Self {
        assert!(
            reserve < self.deadline,
            "fallback reserve must be smaller than the latency deadline"
        );
        self.fallback_reserve = reserve;
        self
    }

    /// How long a cached response may be replayed. The `Cache` trait stores
    /// whole seconds; panics below one second.
    pub fn cache_ttl(mut self, ttl: Duration) -> Self {
        assert!(ttl >= Duration::from_secs(1), "cache TTL must be at least 1s");
        self.cache_ttl = ttl;
        self
    }

    /// Maximum number of timed-out handlers allowed to keep running in the
    /// background to warm the cache. `0` disables background completion.
    pub fn background_limit(mut self, limit: usize) -> Self {
        self.background = Arc::new(Semaphore::new(limit));
        self
    }

    /// Maximum exact response body size that may be cached. Unknown-length and
    /// streaming bodies are never cached.
    pub fn max_cache_body(mut self, bytes: usize) -> Self {
        self.max_cache_body = bytes;
        self
    }

    /// Cumulative counters and latency percentiles for this configuration.
    pub fn stats(&self) -> LatencyStats {
        self.stats.clone()
    }

    fn handler_budget(&self) -> Duration {
        self.deadline.saturating_sub(self.fallback_reserve)
    }
}

/// Cumulative latency statistics. Cloneable and safe to share across tasks.
#[derive(Clone, Default)]
pub struct LatencyStats {
    inner: Arc<StatsInner>,
}

struct StatsInner {
    requests: AtomicU64,
    handler_timeouts: AtomicU64,
    fallback_hits: AtomicU64,
    fallback_misses: AtomicU64,
    cache_stores: AtomicU64,
    cache_store_errors: AtomicU64,
    cache_read_errors: AtomicU64,
    cache_shed: AtomicU64,
    max_us: AtomicU64,
    buckets: [AtomicU64; BUCKETS],
}

impl Default for StatsInner {
    fn default() -> Self {
        Self {
            requests: AtomicU64::new(0),
            handler_timeouts: AtomicU64::new(0),
            fallback_hits: AtomicU64::new(0),
            fallback_misses: AtomicU64::new(0),
            cache_stores: AtomicU64::new(0),
            cache_store_errors: AtomicU64::new(0),
            cache_read_errors: AtomicU64::new(0),
            cache_shed: AtomicU64::new(0),
            max_us: AtomicU64::new(0),
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

/// A point-in-time view of [`LatencyStats`].
///
/// `p50_us` / `p99_us` are upper bounds from 100µs buckets; `max_us` is exact.
/// Counters are cumulative since process start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LatencySnapshot {
    pub requests: u64,
    pub handler_timeouts: u64,
    pub fallback_hits: u64,
    pub fallback_misses: u64,
    pub cache_stores: u64,
    pub cache_store_errors: u64,
    pub cache_read_errors: u64,
    pub cache_shed: u64,
    pub p50_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

impl LatencyStats {
    /// Read the current counters. Counters never reset.
    pub fn snapshot(&self) -> LatencySnapshot {
        let inner = &self.inner;
        let requests = inner.requests.load(Ordering::Relaxed);
        LatencySnapshot {
            requests,
            handler_timeouts: inner.handler_timeouts.load(Ordering::Relaxed),
            fallback_hits: inner.fallback_hits.load(Ordering::Relaxed),
            fallback_misses: inner.fallback_misses.load(Ordering::Relaxed),
            cache_stores: inner.cache_stores.load(Ordering::Relaxed),
            cache_store_errors: inner.cache_store_errors.load(Ordering::Relaxed),
            cache_read_errors: inner.cache_read_errors.load(Ordering::Relaxed),
            cache_shed: inner.cache_shed.load(Ordering::Relaxed),
            p50_us: percentile(&inner.buckets, requests, 50),
            p99_us: percentile(&inner.buckets, requests, 99),
            max_us: inner.max_us.load(Ordering::Relaxed),
        }
    }

    fn record(&self, elapsed: Duration) {
        let us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
        self.inner.requests.fetch_add(1, Ordering::Relaxed);
        self.inner.max_us.fetch_max(us, Ordering::Relaxed);
        self.inner.buckets[bucket_index(us)].fetch_add(1, Ordering::Relaxed);
    }

    fn handler_timeout(&self) {
        self.inner.handler_timeouts.fetch_add(1, Ordering::Relaxed);
    }
    fn fallback_hit(&self) {
        self.inner.fallback_hits.fetch_add(1, Ordering::Relaxed);
    }
    fn fallback_miss(&self) {
        self.inner.fallback_misses.fetch_add(1, Ordering::Relaxed);
    }
    fn cache_store(&self, ok: bool) {
        let counter = if ok { &self.inner.cache_stores } else { &self.inner.cache_store_errors };
        counter.fetch_add(1, Ordering::Relaxed);
    }
    fn cache_read_error(&self) {
        self.inner.cache_read_errors.fetch_add(1, Ordering::Relaxed);
    }
    fn cache_shed(&self) {
        self.inner.cache_shed.fetch_add(1, Ordering::Relaxed);
    }
}

fn bucket_index(us: u64) -> usize {
    ((us / BUCKET_US) as usize).min(BUCKETS - 1)
}

fn percentile(buckets: &[AtomicU64; BUCKETS], total: u64, pct: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    let target = total.saturating_mul(pct).saturating_add(99) / 100;
    let mut seen = 0;
    for (index, bucket) in buckets.iter().enumerate() {
        seen += bucket.load(Ordering::Relaxed);
        if seen >= target {
            return (index as u64 + 1).saturating_mul(BUCKET_US);
        }
    }
    u64::MAX
}

/// axum middleware installed by [`Router::latency`](crate::Router::latency).
pub(crate) async fn middleware(
    State(config): State<LatencyConfig>,
    request: Request,
    next: Next,
) -> Response {
    if !matches!(request.method(), &Method::GET | &Method::HEAD) {
        return next.run(request).await;
    }

    let started = Instant::now();
    let is_head = request.method().as_str() == "HEAD";
    let key = cache_entry::request_cacheable(&request).then(|| cache_entry::cache_key(&request));

    let mut handler = Box::pin(next.run(request));
    match timeout(config.handler_budget(), handler.as_mut()).await {
        Ok(response) => {
            let response = cache_success(&config, response, key.as_deref(), is_head).await;
            config.stats.record(started.elapsed());
            response
        }
        Err(_) => {
            config.stats.handler_timeout();

            // Keep the timed-out handler alive only for a GET that could warm
            // a cache entry, and only within the bounded background budget.
            if !is_head && let Some(key) = key.clone() {
                match config.background.clone().try_acquire_owned() {
                    Ok(permit) => {
                        let config = config.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            let response = handler.await;
                            let _ = cache_success(&config, response, Some(&key), false).await;
                        });
                    }
                    Err(_) => config.stats.cache_shed(),
                }
            }

            let response = fallback(&config, key.as_deref(), is_head).await;
            config.stats.record(started.elapsed());
            response
        }
    }
}

async fn fallback(config: &LatencyConfig, key: Option<&str>, is_head: bool) -> Response {
    let Some(key) = key else {
        config.stats.fallback_miss();
        return deadline_response();
    };
    // Always reserve the fallback window. `tokio` timers can fire late under
    // scheduler pressure; refusing the lookup because the nominal deadline
    // already passed would turn an available stale entry into a 504.
    match timeout(config.fallback_reserve, config.cache.get(key)).await {
        Ok(Ok(Some(bytes))) => match cache_entry::decode(&bytes) {
            Some(entry) if entry.is_fresh() => {
                config.stats.fallback_hit();
                cache_entry::replay(entry, is_head)
            }
            Some(_) => {
                config.stats.fallback_miss();
                deadline_response()
            }
            None => {
                config.stats.cache_read_error();
                config.stats.fallback_miss();
                deadline_response()
            }
        },
        Ok(Ok(None)) => {
            config.stats.fallback_miss();
            deadline_response()
        }
        Ok(Err(_)) => {
            config.stats.cache_read_error();
            config.stats.fallback_miss();
            deadline_response()
        }
        Err(_) => {
            // The lookup itself consumed the reserve.
            config.stats.fallback_miss();
            deadline_response()
        }
    }
}

/// Cache an eligible successful response. The caller has already checked the
/// request key; `is_head` suppresses storage while still allowing the response
/// through untouched.
async fn cache_success(
    config: &LatencyConfig,
    response: Response,
    key: Option<&str>,
    is_head: bool,
) -> Response {
    let Some(key) = key else { return response };
    if is_head || !cache_entry::response_cacheable(&response) {
        return response;
    }
    let Some(size) = response.body().size_hint().exact() else {
        return response;
    };
    if size > config.max_cache_body as u64 {
        return response;
    }

    let ttl_secs = cache_entry::response_max_age(response.headers())
        .map_or(config.cache_ttl.as_secs(), |max_age| config.cache_ttl.as_secs().min(max_age));
    if ttl_secs == 0 {
        return response;
    }

    let (parts, body) = response.into_parts();
    let bytes = match to_bytes(body, config.max_cache_body).await {
        Ok(bytes) => bytes,
        Err(_) => {
            config.stats.cache_store(false);
            return internal_error_response();
        }
    };
    let encoded = cache_entry::encode(
        parts.status,
        &parts.headers,
        &bytes,
        ttl_secs.min(u64::from(u32::MAX)) as u32,
    );
    let response = Response::from_parts(parts, Body::from(bytes));

    let Some(encoded) = encoded else {
        config.stats.cache_store(false);
        return response;
    };
    let cache = config.cache.clone();
    let stats = config.stats.clone();
    let key = key.to_owned();
    tokio::spawn(async move {
        let ok = cache.set(&key, encoded, Some(ttl_secs.max(1))).await.is_ok();
        stats.cache_store(ok);
    });
    response
}

fn deadline_response() -> Response {
    Response::builder()
        .status(StatusCode::GATEWAY_TIMEOUT)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("x-bee-deadline", "exceeded")
        .header(CACHE_CONTROL, "no-store")
        .body(Body::from("request timed out"))
        .expect("static response is valid")
}

fn internal_error_response() -> Response {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(CACHE_CONTROL, "no-store")
        .body(Body::from("response body could not be buffered"))
        .expect("static response is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats_with(samples: &[u64]) -> LatencyStats {
        let stats = LatencyStats::default();
        for us in samples {
            stats.record(Duration::from_micros(*us));
        }
        stats
    }

    #[test]
    fn empty_stats_snapshot_is_zero() {
        let snapshot = LatencyStats::default().snapshot();
        assert_eq!(snapshot.requests, 0);
        assert_eq!(snapshot.p50_us, 0);
        assert_eq!(snapshot.p99_us, 0);
        assert_eq!(snapshot.max_us, 0);
    }

    #[test]
    fn histogram_reports_upper_bounds_and_exact_max() {
        let stats = stats_with(&[1, 1, 2, 3, 4, 10, 100]);
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.requests, 7);
        assert_eq!(snapshot.max_us, 100);
        assert!(snapshot.p50_us >= 100, "{snapshot:?}");
        assert!(snapshot.p99_us >= 100, "{snapshot:?}");
        assert!(snapshot.p99_us <= 200, "{snapshot:?}");
    }

    #[test]
    fn counter_helpers_are_cumulative() {
        let stats = LatencyStats::default();
        stats.handler_timeout();
        stats.fallback_hit();
        stats.fallback_miss();
        stats.cache_store(true);
        stats.cache_store(false);
        stats.cache_read_error();
        stats.cache_shed();
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.handler_timeouts, 1);
        assert_eq!(snapshot.fallback_hits, 1);
        assert_eq!(snapshot.fallback_misses, 1);
        assert_eq!(snapshot.cache_stores, 1);
        assert_eq!(snapshot.cache_store_errors, 1);
        assert_eq!(snapshot.cache_read_errors, 1);
        assert_eq!(snapshot.cache_shed, 1);
    }

    #[test]
    fn config_builders_keep_the_reserve() {
        let cache: Arc<dyn Cache> = Arc::new(bee_cache::MemoryCache::new());
        let config = LatencyConfig::new(cache).deadline(Duration::from_millis(20));
        assert_eq!(config.handler_budget(), Duration::from_millis(18));
        assert_eq!(config.cache_ttl, Duration::from_secs(60));
    }
}
