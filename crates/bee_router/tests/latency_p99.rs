// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Release-only P99 evidence for the latency layer.
//!
//! These tests are ignored by default because wall-clock timing on a shared CI
//! runner is not a reliable gate. Build the router's default 9ms budget and
//! run on the target machine:
//!
//! ```text
//! cargo test -p bee_router --release --test latency_p99 -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::header::CACHE_CONTROL;
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use bee_cache::{Cache, MemoryCache};
use bee_router::Router;
use bee_router::latency::{LatencyConfig, LatencySnapshot, LatencyStats};
use tower::ServiceExt;

#[derive(Clone)]
struct State {
    slow: Arc<AtomicBool>,
}

async fn public(axum::extract::State(state): axum::extract::State<State>) -> Response {
    if state.slow.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    ([(CACHE_CONTROL, "public, max-age=60")], "cached").into_response()
}

fn app_with(
    max_cache_body: usize,
    background_limit: usize,
) -> (axum::Router, LatencyStats, Arc<AtomicBool>) {
    let slow = Arc::new(AtomicBool::new(false));
    let state = State { slow: Arc::clone(&slow) };
    let cache: Arc<dyn Cache> = Arc::new(MemoryCache::new());
    let config =
        LatencyConfig::new(cache).max_cache_body(max_cache_body).background_limit(background_limit);
    let stats = config.stats();
    let app =
        Router::new().latency(config).ns("/api", |ns| ns.get("/public", public)).with_state(state);
    (app, stats, slow)
}

async fn get(app: &axum::Router) -> Duration {
    let started = Instant::now();
    let response = app
        .clone()
        .oneshot(
            Request::builder().method(Method::GET).uri("/api/public").body(Body::empty()).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // Drain the body so the measurement covers response completion, not just
    // headers.
    let _ = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
    started.elapsed()
}

fn percentile(samples: &mut [Duration], pct: u64) -> Duration {
    samples.sort_unstable();
    let index = ((samples.len() as u64 * pct) / 100).min(samples.len() as u64 - 1) as usize;
    samples[index]
}

async fn wait_for_cache(stats: &LatencyStats) {
    for _ in 0..200 {
        if stats.snapshot().cache_stores > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("cache never warmed: {:?}", stats.snapshot());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "release-only latency harness"]
async fn p99_fast_path_is_under_10ms() {
    if cfg!(debug_assertions) {
        eprintln!("skipping P99 fast-path measurement in a debug build");
        return;
    }
    // Avoid 5,000 concurrent cache writes: this measures the deadline layer,
    // not the async store path.
    let (app, stats, _) = app_with(0, 128);
    let mut samples = Vec::with_capacity(5_000);
    for _ in 0..5_000 {
        samples.push(get(&app).await);
    }
    let p50 = percentile(&mut samples, 50);
    let p99 = percentile(&mut samples, 99);
    let max = samples.last().copied().unwrap_or_default();
    eprintln!("fast path: p50={p50:?} p99={p99:?} max={max:?} stats={:?}", stats.snapshot());
    assert!(p99 < Duration::from_millis(10), "p99={p99:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "release-only latency harness"]
async fn p99_fallback_path_is_under_10ms() {
    if cfg!(debug_assertions) {
        eprintln!("skipping P99 fallback-path measurement in a debug build");
        return;
    }
    // No background completion here: this measures the fallback path without
    // competing cache writers. Behavior tests cover background warming.
    let (app, stats, slow) = app_with(64 * 1024, 0);
    let _ = get(&app).await;
    wait_for_cache(&stats).await;
    slow.store(true, Ordering::SeqCst);

    let mut samples = Vec::with_capacity(500);
    for _ in 0..500 {
        samples.push(get(&app).await);
    }
    let p50 = percentile(&mut samples, 50);
    let p99 = percentile(&mut samples, 99);
    let max = samples.last().copied().unwrap_or_default();
    let snapshot: LatencySnapshot = stats.snapshot();
    eprintln!("fallback path: p50={p50:?} p99={p99:?} max={max:?} stats={snapshot:?}");
    assert!(snapshot.fallback_hits >= 500, "{snapshot:?}");
    assert!(p99 < Duration::from_millis(10), "p99={p99:?}");
}
