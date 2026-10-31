// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use std::time::Duration;

use axum::Router as AxumRouter;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;

/// 统一错误信封 `{code,msg,data}`，保证 /api/v1 的错误响应也是 JSON。
fn error_response(status: StatusCode, msg: &'static str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({
            "code": status.as_u16(),
            "msg": msg,
            "data": serde_json::Value::Null,
        })),
    )
        .into_response()
}

/// A declarative router that collects named routes and builds an
/// [`axum::Router`]. Routes are organised into [`RouteGroup`]s via
/// the [`ns`](Router::ns) method.
///
/// The state type `S` is the router's shared state, passed to handlers
/// through axum extractors such as [`axum::extract::State`]. Stateless
/// usage keeps the default `S = ()`.
pub struct Router<S = ()> {
    routes: Vec<(String, MethodRouter<S>)>,
    body_limit: usize,
}

impl<S> Router<S> {
    pub fn new() -> Self {
        Self { routes: Vec::new(), body_limit: 2 * 1024 * 1024 }
    }

    pub fn ns<F>(mut self, prefix: &str, f: F) -> Self
    where
        F: FnOnce(RouteGroup<S>) -> RouteGroup<S>,
    {
        let group = RouteGroup::new(prefix.to_string());
        let group = f(group);
        self.routes.extend(group.routes);
        self
    }

    /// 设置请求体上限（字节），默认 2 MiB。
    pub fn body_limit(mut self, bytes: usize) -> Self {
        self.body_limit = bytes;
        self
    }

    /// Build the router without supplying state. Handlers using
    /// [`axum::extract::State`] require [`Router::with_state`] instead.
    ///
    /// Every route gets a 2 MiB body limit (see [`Router::body_limit`]) and a
    /// 30 s request timeout.
    pub fn build(self) -> AxumRouter<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        let body_limit = self.body_limit;
        let mut router = AxumRouter::new();
        for (path, method_router) in self.routes {
            router = router.route(&path, method_router);
        }
        router.layer(
            // One stack per `Router::layer` call: the error handling must be
            // inside it so the composed service's error is `Infallible`.
            tower::ServiceBuilder::new()
                .layer(axum::error_handling::HandleErrorLayer::new(
                    |err: axum::BoxError| async move {
                        if err.is::<tower::timeout::error::Elapsed>() {
                            error_response(StatusCode::REQUEST_TIMEOUT, "请求超时")
                        } else {
                            error_response(StatusCode::INTERNAL_SERVER_ERROR, "服务器内部错误")
                        }
                    },
                ))
                .layer(tower::timeout::TimeoutLayer::new(Duration::from_secs(30)))
                .layer(axum::extract::DefaultBodyLimit::max(body_limit)),
        )
    }

    /// Build the router and attach `state`, yielding a fully constructed
    /// [`AxumRouter<()>`](axum::Router) ready to serve.
    pub fn with_state(self, state: S) -> AxumRouter<()>
    where
        S: Clone + Send + Sync + 'static,
    {
        self.build().with_state(state)
    }
}

impl<S> Default for Router<S> {
    fn default() -> Self {
        Self::new()
    }
}

/// A grouping of routes sharing a common URL prefix, created via
/// [`Router::ns`]. Supports get, post, put, and delete.
pub struct RouteGroup<S = ()> {
    prefix: String,
    routes: Vec<(String, MethodRouter<S>)>,
}

impl<S> RouteGroup<S> {
    pub fn new(prefix: String) -> Self {
        Self { prefix, routes: Vec::new() }
    }

    pub fn get<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, S>,
        T: 'static,
        S: Clone + Send + Sync + 'static,
    {
        let full = format!("{}{}", self.prefix, path);
        self.routes.push((full, axum::routing::get(handler)));
        self
    }

    pub fn post<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, S>,
        T: 'static,
        S: Clone + Send + Sync + 'static,
    {
        let full = format!("{}{}", self.prefix, path);
        self.routes.push((full, axum::routing::post(handler)));
        self
    }

    pub fn put<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, S>,
        T: 'static,
        S: Clone + Send + Sync + 'static,
    {
        let full = format!("{}{}", self.prefix, path);
        self.routes.push((full, axum::routing::put(handler)));
        self
    }

    pub fn delete<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, S>,
        T: 'static,
        S: Clone + Send + Sync + 'static,
    {
        let full = format!("{}{}", self.prefix, path);
        self.routes.push((full, axum::routing::delete(handler)));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::{Query, State};
    use axum::http::{Request, StatusCode};
    use serde::Deserialize;
    use tower::ServiceExt;

    #[derive(Deserialize)]
    struct HelloParams {
        name: Option<String>,
    }

    async fn hello(Query(params): Query<HelloParams>) -> String {
        format!("hello {}", params.name.unwrap_or_else(|| "world".into()))
    }

    #[tokio::test]
    async fn stateless_handler_with_query_extractor() {
        let app = Router::new().ns("/api", |ns| ns.get("/hello", hello));
        let res = app
            .build()
            .oneshot(Request::builder().uri("/api/hello?name=rust").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"hello rust");
    }

    #[derive(Clone)]
    struct AppState {
        counter: usize,
    }

    async fn count(State(state): State<AppState>) -> String {
        format!("count={}", state.counter)
    }

    #[tokio::test]
    async fn handler_with_state_extractor() {
        let app = Router::new()
            .ns("/api", |ns| ns.get("/count", count))
            .with_state(AppState { counter: 42 });
        let res = app
            .oneshot(Request::builder().uri("/api/count").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"count=42");
    }

    #[tokio::test]
    async fn error_response_uses_json_envelope() {
        for (status, msg) in [
            (StatusCode::REQUEST_TIMEOUT, "请求超时"),
            (StatusCode::INTERNAL_SERVER_ERROR, "服务器内部错误"),
        ] {
            let res = error_response(status, msg);
            assert_eq!(res.status(), status);
            assert_eq!(
                res.headers()[axum::http::header::CONTENT_TYPE].to_str().unwrap(),
                "application/json"
            );
            let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                json,
                serde_json::json!({ "code": status.as_u16(), "msg": msg, "data": null })
            );
        }
    }

    async fn body_len(body: axum::body::Bytes) -> String {
        body.len().to_string()
    }

    #[tokio::test]
    async fn body_limit_defaults_to_2_mib_and_is_configurable() {
        assert_eq!(Router::<()>::new().body_limit, 2 * 1024 * 1024);

        let app = Router::new().body_limit(4).ns("/api", |ns| ns.post("/len", body_len));
        let res = app
            .build()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/len")
                    .body(Body::from("hello"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let app = Router::new().body_limit(8).ns("/api", |ns| ns.post("/len", body_len));
        let res = app
            .build()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/len")
                    .body(Body::from("hello"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"5");
    }
}
