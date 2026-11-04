// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! HTTP-level tests for the opt-in latency budget and stale-cache fallback.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Router as AxumRouter;
use axum::body::{Body, to_bytes};
use axum::http::header::{ACCEPT, AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, SET_COOKIE, VARY};
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use bee_cache::{Cache, MemoryCache};
use bee_router::Router;
use bee_router::latency::{LatencyConfig, LatencySnapshot, LatencyStats};
use tower::ServiceExt;

#[derive(Clone, Default)]
struct TestState {
    slow: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl TestState {
    fn slow(&self) -> bool {
        self.slow.load(Ordering::SeqCst)
    }

    fn set_slow(&self, slow: bool) {
        self.slow.store(slow, Ordering::SeqCst);
    }
}

async fn public(axum::extract::State(state): axum::extract::State<TestState>) -> Response {
    state.calls.fetch_add(1, Ordering::SeqCst);
    if state.slow() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    ([(CACHE_CONTROL, "public, max-age=60")], "cached".to_string()).into_response()
}

async fn private(axum::extract::State(state): axum::extract::State<TestState>) -> Response {
    if state.slow() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    ([(CACHE_CONTROL, "private, max-age=60")], "private").into_response()
}

async fn with_cookie(axum::extract::State(state): axum::extract::State<TestState>) -> Response {
    if state.slow() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    ([(CACHE_CONTROL, "public, max-age=60"), (SET_COOKIE, "session=abc; Path=/")], "cookie")
        .into_response()
}

async fn with_vary(axum::extract::State(state): axum::extract::State<TestState>) -> Response {
    if state.slow() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    ([(CACHE_CONTROL, "public, max-age=60"), (VARY, "accept")], "vary").into_response()
}

async fn not_ok(axum::extract::State(state): axum::extract::State<TestState>) -> Response {
    if state.slow() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    (StatusCode::INTERNAL_SERVER_ERROR, [(CACHE_CONTROL, "public")], "error").into_response()
}

async fn large(axum::extract::State(state): axum::extract::State<TestState>) -> Response {
    if state.slow() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    ([(CACHE_CONTROL, "public, max-age=60")], "x".repeat(65 * 1024)).into_response()
}

async fn write(axum::extract::State(state): axum::extract::State<TestState>) -> Response {
    state.calls.fetch_add(1, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(150)).await;
    (StatusCode::OK, "written").into_response()
}

fn app(state: TestState, cache: Arc<dyn Cache>) -> (AxumRouter, LatencyStats) {
    let config = LatencyConfig::new(cache)
        .deadline(Duration::from_millis(100))
        .fallback_reserve(Duration::from_millis(20));
    let stats = config.stats();
    let app = Router::new()
        .latency(config)
        .ns("/api", |ns| {
            ns.get("/public", public)
                .get("/private", private)
                .get("/cookie", with_cookie)
                .get("/vary", with_vary)
                .get("/error", not_ok)
                .get("/large", large)
                .get("/a", public)
                .get("/b", public)
                .post("/write", write)
        })
        .with_state(state);
    (app, stats)
}

fn cache() -> Arc<dyn Cache> {
    Arc::new(MemoryCache::new())
}

async fn call(app: &AxumRouter, method: Method, uri: &str) -> Response {
    app.clone()
        .oneshot(Request::builder().method(method).uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn call_with(
    app: &AxumRouter,
    method: Method,
    uri: &str,
    name: &str,
    value: &str,
) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(name, value)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn body_string(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn wait_for(stats: &LatencyStats, predicate: impl Fn(&LatencySnapshot) -> bool, what: &str) {
    for _ in 0..200 {
        if predicate(&stats.snapshot()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}: {:?}", stats.snapshot());
}

#[tokio::test]
async fn fast_get_returns_and_stores_a_public_response() {
    let (app, stats) = app(TestState::default(), cache());
    let response = call(&app, Method::GET, "/api/public").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_string(response).await, "cached");
    wait_for(&stats, |s| s.cache_stores == 1, "cache store").await;
    assert_eq!(stats.snapshot().requests, 1);
}

#[tokio::test]
async fn slow_get_replays_the_stale_cache_entry() {
    let state = TestState::default();
    let (app, stats) = app(state.clone(), cache());
    let warm = call(&app, Method::GET, "/api/public").await;
    assert_eq!(warm.status(), StatusCode::OK);
    wait_for(&stats, |s| s.cache_stores == 1, "cache store").await;

    state.set_slow(true);
    let response = call(&app, Method::GET, "/api/public").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("x-bee-cache").unwrap(), "stale");
    assert!(response.headers().get("age").is_some());
    assert_eq!(body_string(response).await, "cached");
    assert_eq!(stats.snapshot().fallback_hits, 1);
}

#[tokio::test]
async fn slow_get_without_cache_returns_504() {
    let state = TestState::default();
    state.set_slow(true);
    let (app, stats) = app(state, cache());
    let response = call(&app, Method::GET, "/api/public").await;
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(response.headers().get("x-bee-deadline").unwrap(), "exceeded");
    assert_eq!(body_string(response).await, "request timed out");
    let snapshot = stats.snapshot();
    assert_eq!(snapshot.handler_timeouts, 1);
    assert_eq!(snapshot.fallback_misses, 1);
}

#[tokio::test]
async fn timed_out_handler_warms_the_cache_for_the_next_request() {
    let state = TestState::default();
    state.set_slow(true);
    let (app, stats) = app(state, cache());

    let first = call(&app, Method::GET, "/api/public").await;
    assert_eq!(first.status(), StatusCode::GATEWAY_TIMEOUT);
    wait_for(&stats, |s| s.cache_stores == 1, "background cache store").await;

    let second = call(&app, Method::GET, "/api/public").await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(second.headers().get("x-bee-cache").unwrap(), "stale");
    assert_eq!(body_string(second).await, "cached");
}

#[tokio::test]
async fn writes_keep_the_existing_timeout_and_are_not_recorded() {
    let (app, stats) = app(TestState::default(), cache());
    let started = Instant::now();
    let response = call(&app, Method::POST, "/api/write").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(started.elapsed() >= Duration::from_millis(140));
    assert_eq!(body_string(response).await, "written");
    assert_eq!(stats.snapshot().requests, 0);
}

#[tokio::test]
async fn head_replays_a_get_entry_without_a_body_or_a_store() {
    let state = TestState::default();
    let (app, stats) = app(state.clone(), cache());
    let warm = call(&app, Method::GET, "/api/public").await;
    assert_eq!(warm.status(), StatusCode::OK);
    wait_for(&stats, |s| s.cache_stores == 1, "cache store").await;

    state.set_slow(true);
    let response = call(&app, Method::HEAD, "/api/public").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("x-bee-cache").unwrap(), "stale");
    assert_eq!(response.headers().get(CONTENT_LENGTH).unwrap(), "6");
    assert!(body_string(response).await.is_empty());
    assert_eq!(stats.snapshot().cache_stores, 1);
}

#[tokio::test]
async fn non_public_responses_are_not_cached() {
    for uri in ["/api/private", "/api/cookie", "/api/vary", "/api/error", "/api/large"] {
        let state = TestState::default();
        state.set_slow(true);
        let (app, stats) = app(state, cache());
        let response = call(&app, Method::GET, uri).await;
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT, "{uri}");
        assert_eq!(stats.snapshot().cache_stores, 0, "{uri}");
    }
}

#[tokio::test]
async fn credential_and_explicit_bypass_requests_do_not_use_the_cache() {
    let state = TestState::default();
    let (app, stats) = app(state.clone(), cache());
    let warm = call(&app, Method::GET, "/api/public").await;
    assert_eq!(warm.status(), StatusCode::OK);
    wait_for(&stats, |s| s.cache_stores == 1, "cache store").await;

    state.set_slow(true);
    for (name, value) in [
        (AUTHORIZATION.as_str(), "Bearer token"),
        ("range", "bytes=0-1"),
        (CACHE_CONTROL.as_str(), "no-store"),
        (CACHE_CONTROL.as_str(), "no-cache"),
    ] {
        let response = call_with(&app, Method::GET, "/api/public", name, value).await;
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT, "{name}: {value}");
    }
}

#[tokio::test]
async fn cache_keys_isolate_routes_and_accept_headers() {
    let state = TestState::default();
    let (app, stats) = app(state.clone(), cache());
    let warm = call(&app, Method::GET, "/api/a").await;
    assert_eq!(warm.status(), StatusCode::OK);
    wait_for(&stats, |s| s.cache_stores == 1, "cache store").await;

    state.set_slow(true);
    let other = call(&app, Method::GET, "/api/b").await;
    assert_eq!(other.status(), StatusCode::GATEWAY_TIMEOUT);
    let accept = call_with(&app, Method::GET, "/api/a", ACCEPT.as_str(), "application/json").await;
    assert_eq!(accept.status(), StatusCode::GATEWAY_TIMEOUT);

    let same = call(&app, Method::GET, "/api/a").await;
    assert_eq!(same.status(), StatusCode::OK);
    assert_eq!(same.headers().get("x-bee-cache").unwrap(), "stale");
}

#[tokio::test]
async fn stats_expose_latency_percentiles() {
    let (app, stats) = app(TestState::default(), cache());
    let response = call(&app, Method::GET, "/api/public").await;
    assert_eq!(response.status(), StatusCode::OK);
    let snapshot = stats.snapshot();
    assert_eq!(snapshot.requests, 1);
    assert!(snapshot.p99_us > 0);
    assert!(snapshot.max_us > 0);
    assert!(snapshot.p99_us >= snapshot.p50_us);
}
