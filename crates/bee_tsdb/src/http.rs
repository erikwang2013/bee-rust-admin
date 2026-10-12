// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
// Shared by all three HTTP-based backends (influxdb/iotdb/questdb).
// It lives here rather than inside one backend module because each backend
// must compile on its own: while it lived in the influxdb module, enabling
// only iotdb/questdb failed with `unresolved import crate::influxdb`.

use reqwest::Client;
use std::time::Duration;

/// Shared HTTP client: 30s request timeout, 5s connect timeout, so a hung
/// backend cannot stall a request forever.
pub(crate) fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| Client::new())
}
