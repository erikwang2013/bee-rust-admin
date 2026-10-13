// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
// Shared by all three HTTP-based backends (neo4j/nebulagraph/arangodb).
// It lives here rather than inside one backend module because each backend
// must compile on its own: while it lived in the neo4j module, enabling
// only nebulagraph/arangodb failed with `unresolved import crate::neo4j`.

use reqwest::Client;
use std::time::Duration;

use crate::GraphError;

/// Shared HTTP client: 30s request timeout, 5s connect timeout, so a hung
/// backend cannot stall a request forever.
pub(crate) fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Failed request transport → [`GraphError::ConnectionError`].
pub(crate) fn conn_err(e: reqwest::Error) -> GraphError {
    GraphError::ConnectionError(e.to_string())
}

/// Failed body decode → [`GraphError::QueryError`].
pub(crate) fn query_err(e: reqwest::Error) -> GraphError {
    GraphError::QueryError(e.to_string())
}

/// Non-2xx response → [`GraphError::QueryError`] naming `backend` and `op`.
pub(crate) async fn http_error(res: reqwest::Response, backend: &str, op: &str) -> GraphError {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    GraphError::QueryError(format!("{backend} {op} failed with {status}: {body}"))
}

/// Escapes a backtick-quoted identifier (Cypher labels, nGQL labels).
#[cfg(any(feature = "neo4j", feature = "nebulagraph"))]
pub(crate) fn backtick(s: &str) -> String {
    s.replace('`', "``")
}

/// Escapes a string inside a double-quoted literal (nGQL / AQL).
#[cfg(any(feature = "nebulagraph", feature = "arangodb"))]
pub(crate) fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
