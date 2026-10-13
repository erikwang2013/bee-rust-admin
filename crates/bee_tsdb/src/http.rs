// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
// Shared by all three HTTP-based backends (influxdb/iotdb/questdb).
// It lives here rather than inside one backend module because each backend
// must compile on its own: while it lived in the influxdb module, enabling
// only iotdb/questdb failed with `unresolved import crate::influxdb`.

use reqwest::Client;
use std::time::Duration;

use crate::TsdbError;

/// Shared HTTP client: 30s request timeout, 5s connect timeout, so a hung
/// backend cannot stall a request forever.
pub(crate) fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Failed request transport → [`TsdbError::ConnectionError`].
pub(crate) fn conn_err(e: reqwest::Error) -> TsdbError {
    TsdbError::ConnectionError(e.to_string())
}

/// Failed body decode → [`TsdbError::QueryError`].
pub(crate) fn query_err(e: reqwest::Error) -> TsdbError {
    TsdbError::QueryError(e.to_string())
}

/// Non-2xx response → [`TsdbError::QueryError`] naming `backend` and `op`.
pub(crate) async fn http_error(res: reqwest::Response, backend: &str, op: &str) -> TsdbError {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    TsdbError::QueryError(format!("{backend} {op} failed with {status}: {body}"))
}

/// [`http_error`] as a `Result` guard for callers that only need the check.
#[cfg(feature = "influxdb")]
pub(crate) async fn check_status(
    res: reqwest::Response,
    backend: &str,
    op: &str,
) -> Result<(), TsdbError> {
    if res.status().is_success() { Ok(()) } else { Err(http_error(res, backend, op).await) }
}
