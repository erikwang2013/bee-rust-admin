// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//
// HTTP-layer integration tests for `bee_router`. Routes are exercised
// through the real axum stack built by `Router::build()` via
// `tower::ServiceExt::oneshot` (in-process, no socket).
//
// The security-filter scenarios in the `security_http` module are gated on
// the `security` feature (SecurityFilter lives behind it); run
// `cargo test -p bee_router --features security` to include them.

use axum::body::{Body, to_bytes};
use axum::extract::{Json, Query};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;

#[derive(Deserialize)]
struct HelloParams {
    name: Option<String>,
}

async fn health() -> &'static str {
    "OK"
}

async fn hello(Query(params): Query<HelloParams>) -> String {
    format!("hello {}", params.name.unwrap_or_else(|| "world".into()))
}

#[derive(Deserialize, Serialize)]
struct Submit {
    message: String,
}

async fn submit(Json(body): Json<Submit>) -> Json<Submit> {
    Json(body)
}

/// Mirrors examples/hello's route table: a single `/api/v1` namespace.
fn app() -> axum::Router {
    bee_router::Router::new()
        .ns("/api/v1", |ns| ns.get("/health", health).get("/hello", hello).post("/submit", submit))
        .build()
}

async fn call(uri: &str) -> Response {
    app().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap()
}

async fn body_string(res: Response) -> String {
    let bytes = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn health_endpoint_returns_200_ok() {
    let res = call("/api/v1/health").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_string(res).await, "OK");
}

#[tokio::test]
async fn unknown_path_inside_namespace_returns_404() {
    assert_eq!(call("/api/v1/nope").await.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_namespace_returns_404() {
    assert_eq!(call("/api/v2/health").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(call("/other/health").await.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn wrong_method_on_existing_route_returns_405() {
    let res = app()
        .oneshot(
            Request::builder().method("POST").uri("/api/v1/health").body(Body::empty()).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn namespace_prefix_matches_nested_route_with_query() {
    let res = call("/api/v1/hello?name=rust").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_string(res).await, "hello rust");
}

#[tokio::test]
async fn post_route_handles_json_body() {
    let res = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/submit")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"hi"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_string(res).await;
    assert!(body.contains("\"message\""), "body: {body}");
    assert!(body.contains("\"hi\""), "body: {body}");
}

#[tokio::test]
async fn oversized_body_rejected_with_413() {
    let big = vec![0u8; 3 * 1024 * 1024]; // over the 2 MiB build() limit
    let res = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/submit")
                .header("content-type", "application/json")
                .body(Body::from(big))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[cfg(feature = "security")]
mod security_http {
    use super::*;
    use bee_cache::{Cache, MemoryCache};
    use bee_router::filter::Filter;
    use bee_router::{Context, SecurityFilter};
    use bee_session::Session;
    use bee_template::TemplateEngine;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    fn make_context(request: axum::extract::Request) -> Context {
        let cache: Arc<dyn Cache> = Arc::new(MemoryCache::new());
        let session = Session::new(cache, Duration::from_secs(3600));
        let templates =
            Arc::new(TemplateEngine::new(Path::new("tests/fixtures/templates")).unwrap());
        Context::new(request, session, templates)
    }

    /// Handler running the router's security pipeline: `SecurityFilter`
    /// scans the request and aborts with 400 on attack; clean requests
    /// fall through to a normal text response.
    async fn secured(request: axum::extract::Request) -> Response {
        let filter = SecurityFilter::new();
        let mut ctx = make_context(request);
        let _ = filter.before(&mut ctx);
        if ctx.is_aborted() {
            return ctx.into_response();
        }
        ctx.text("ok").unwrap();
        ctx.into_response()
    }

    fn secured_app() -> axum::Router {
        bee_router::Router::new().ns("/api/v1", |ns| ns.get("/search", secured)).build()
    }

    async fn call_secured(uri: &str) -> Response {
        secured_app()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn percent_encoded_xss_in_query_blocked_at_http_layer() {
        let res = call_secured("/api/v1/search?q=%3Cscript%3Ealert(1)%3C/script%3E").await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = body_string(res).await;
        assert!(body.contains("xss"), "body: {body}");
    }

    #[tokio::test]
    async fn percent_encoded_sql_injection_blocked() {
        let res = call_secured("/api/v1/search?q=%27%20OR%201%3D1--").await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = body_string(res).await;
        assert!(body.contains("sql_injection"), "body: {body}");
    }

    #[tokio::test]
    async fn path_traversal_in_query_blocked() {
        let res = call_secured("/api/v1/search?file=%2e%2e%2fetc%2fpasswd").await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = body_string(res).await;
        assert!(body.contains("path_traversal"), "body: {body}");
    }

    #[tokio::test]
    async fn non_utf8_cookie_header_blocked() {
        let res = secured_app()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/search")
                    .header("cookie", axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = body_string(res).await;
        assert!(body.contains("non-UTF-8"), "body: {body}");
    }

    #[tokio::test]
    async fn clean_request_passes_security_pipeline() {
        let res = call_secured("/api/v1/search?q=hello+world").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_string(res).await, "ok");
    }
}
