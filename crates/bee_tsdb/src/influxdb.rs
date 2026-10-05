// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;

/// Shared HTTP client: 30s request timeout, 5s connect timeout, so a hung
/// backend cannot stall a request forever.
pub(crate) fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| Client::new())
}

use crate::{
    Aggregation, CQSpec, Fields, FilterOp, Point, TagFilter, Tags, TimeSeries, TimeSeriesDB,
    Timestamp, TsdbError,
};

/// InfluxDB v2 driver backed by its HTTP (Flux) API.
pub struct InfluxDB {
    client: Client,
    base_url: String,
    bucket: String,
    org: String,
}

impl InfluxDB {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::new_with_credentials(base_url, "bee", "bee")
    }

    pub fn new_with_credentials(
        base_url: impl Into<String>,
        bucket: impl Into<String>,
        org: impl Into<String>,
    ) -> Self {
        Self {
            client: http_client(),
            base_url: base_url.into(),
            bucket: bucket.into(),
            org: org.into(),
        }
    }
}

#[async_trait]
impl TimeSeriesDB for InfluxDB {
    async fn write_point(&self, point: Point) -> Result<(), TsdbError> {
        self.write_batch(vec![point]).await
    }

    async fn write_batch(&self, points: TimeSeries) -> Result<(), TsdbError> {
        let mut body = String::new();
        for p in &points {
            body.push_str(&line_protocol(p)?);
            body.push('\n');
        }
        let url = format!(
            "{}/api/v2/write?bucket={}&org={}&precision=ns",
            self.base_url, self.bucket, self.org
        );
        let res = self.client.post(url).body(body).send().await.map_err(conn_err)?;
        check_status(res, "write").await
    }

    async fn query_range(
        &self,
        measurement: &str,
        start: Timestamp,
        end: Timestamp,
        tag_filters: Option<&[TagFilter]>,
    ) -> Result<TimeSeries, TsdbError> {
        let mut flux = format!(
            "from(bucket: \"{b}\")\n  |> range(start: {s}, stop: {e})\n  |> filter(fn: (r) => r._measurement == \"{m}\")",
            b = flux_str(&self.bucket),
            s = start.to_rfc3339(),
            e = end.to_rfc3339(),
            m = flux_str(measurement),
        );
        if let Some(filters) = tag_filters {
            for f in filters {
                let key = flux_key(&f.key)?;
                let cond = match f.op {
                    FilterOp::Eq => format!("r.{key} == \"{}\"", flux_str(&f.value)),
                    FilterOp::Neq => format!("r.{key} != \"{}\"", flux_str(&f.value)),
                    FilterOp::Regex => format!("r.{key} =~ /{}/", flux_regex(&f.value)),
                };
                flux.push_str(&format!("\n  |> filter(fn: (r) => {cond})"));
            }
        }
        let res = self
            .client
            .post(format!("{}/api/v2/query?org={}", self.base_url, self.org))
            .header("Accept", "application/csv")
            .json(&serde_json::json!({ "query": flux, "type": "flux" }))
            .send()
            .await
            .map_err(conn_err)?;
        if !res.status().is_success() {
            return Err(http_error(res, "query_range").await);
        }
        let body = res.text().await.map_err(query_err)?;
        parse_csv(&body)
    }

    async fn create_continuous_query(&self, spec: CQSpec) -> Result<(), TsdbError> {
        let agg = aggregation_fn(&spec.aggregation);
        let every = flux_duration(&spec.every)?;
        let flux = format!(
            "option task = {{name: \"{n}\", every: {e}, offset: 0s, org: \"{o}\"}}\n\n\
             from(bucket: \"{b}\")\n  |> range(start: -task.every)\n  \
             |> filter(fn: (r) => r._measurement == \"{s}\")\n  \
             |> {a}()\n  |> to(bucket: \"{t}\")",
            n = flux_str(&spec.name),
            e = every,
            o = flux_str(&self.org),
            b = flux_str(&self.bucket),
            s = flux_str(&spec.source),
            a = agg,
            t = flux_str(&spec.target),
        );
        let res = self
            .client
            .post(format!("{}/api/v2/tasks", self.base_url))
            .json(&serde_json::json!({ "flux": flux }))
            .send()
            .await
            .map_err(conn_err)?;
        check_status(res, "create_continuous_query").await
    }
}

fn aggregation_fn(agg: &Aggregation) -> &'static str {
    match agg {
        Aggregation::Mean => "mean",
        Aggregation::Sum => "sum",
        Aggregation::Count => "count",
        Aggregation::Min => "min",
        Aggregation::Max => "max",
    }
}

/// Escapes a string inside a Flux double-quoted literal.
fn flux_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Escapes a value inside a Flux regex literal `/.../`.
fn flux_regex(s: &str) -> String {
    s.replace('\\', "\\\\").replace('/', "\\/")
}

/// Tag keys become bare column references (`r.{key}`); only identifiers are
/// safe there.
fn flux_key(s: &str) -> Result<&str, TsdbError> {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(s)
    } else {
        Err(TsdbError::QueryError(format!("invalid flux tag key: {s:?}")))
    }
}

/// Flux duration literal (e.g. `1m`), used in task options.
fn flux_duration(s: &str) -> Result<&str, TsdbError> {
    let body_len = s.trim_end_matches(|c: char| c.is_ascii_alphabetic()).len();
    let (body, unit) = s.split_at(body_len);
    if !body.is_empty()
        && body.bytes().all(|b| b.is_ascii_digit())
        && matches!(unit, "ns" | "us" | "ms" | "s" | "m" | "h" | "d" | "w")
    {
        Ok(s)
    } else {
        Err(TsdbError::QueryError(format!("invalid flux duration: {s:?}")))
    }
}

fn escape_measurement(s: &str) -> String {
    s.replace(' ', "\\ ").replace(',', "\\,")
}

fn escape_tag(s: &str) -> String {
    escape_measurement(s).replace('=', "\\=")
}

fn field_value(v: &serde_json::Value) -> Result<String, TsdbError> {
    match v {
        serde_json::Value::String(s) => {
            Ok(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        }
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(if *b { "true" } else { "false" }.to_string()),
        _ => Err(TsdbError::WriteError(format!("unsupported field value: {v}"))),
    }
}

fn line_protocol(point: &Point) -> Result<String, TsdbError> {
    let mut line = escape_measurement(&point.measurement);
    let mut tags: Vec<(&String, &String)> = point.tags.iter().collect();
    tags.sort_by_key(|(k, _)| *k);
    for (k, v) in tags {
        line.push_str(&format!(",{}={}", escape_tag(k), escape_tag(v)));
    }
    line.push(' ');
    let mut first = true;
    for (k, v) in &point.fields {
        if !first {
            line.push(',');
        }
        first = false;
        line.push_str(&escape_tag(k));
        line.push('=');
        line.push_str(&field_value(v)?);
    }
    let ts = point.timestamp.timestamp_nanos_opt().ok_or_else(|| {
        TsdbError::WriteError(format!("timestamp out of range: {}", point.timestamp))
    })?;
    Ok(format!("{line} {ts}"))
}

fn parse_value(s: &str) -> serde_json::Value {
    if let Ok(n) = s.parse::<f64>() { serde_json::json!(n) } else { serde_json::json!(s) }
}

fn parse_time(s: &str) -> Result<Timestamp, TsdbError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| TsdbError::QueryError(format!("bad timestamp {s:?}: {e}")))
}

/// Splits a CSV line, honoring double-quoted fields: Flux CSV quotes values
/// containing commas, so a bare `split(',')` would misalign columns.
fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            ',' if !in_quotes => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Parses Flux annotated-CSV output into points. Lines starting with `#` are
/// annotations; the first plain line is the header.
fn parse_csv(body: &str) -> Result<TimeSeries, TsdbError> {
    let mut points = Vec::new();
    let mut header: Option<Vec<String>> = None;
    for line in body.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let cols = split_csv(line);
        match header.as_ref() {
            None => header = Some(cols),
            Some(h) => {
                let mut tags = Tags::new();
                let mut fields = Fields::new();
                let mut measurement = String::new();
                let mut field_name = String::new();
                let mut timestamp = None;
                for (i, col) in cols.iter().enumerate() {
                    match h.get(i).map(|s| s.as_str()).unwrap_or_default() {
                        "_time" => timestamp = Some(parse_time(col)?),
                        "_measurement" => measurement = col.to_string(),
                        "_field" => field_name = col.to_string(),
                        "_value" => {
                            fields.insert(field_name.clone(), parse_value(col));
                        }
                        "result" | "table" | "" => {}
                        name => {
                            tags.insert(name.to_string(), col.to_string());
                        }
                    }
                }
                if let Some(ts) = timestamp {
                    points.push(Point { measurement, tags, fields, timestamp: ts });
                }
            }
        }
    }
    Ok(points)
}

async fn check_status(res: reqwest::Response, op: &str) -> Result<(), TsdbError> {
    if res.status().is_success() { Ok(()) } else { Err(http_error(res, op).await) }
}

async fn http_error(res: reqwest::Response, op: &str) -> TsdbError {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    TsdbError::QueryError(format!("influxdb {op} failed with {status}: {body}"))
}

fn conn_err(e: reqwest::Error) -> TsdbError {
    TsdbError::ConnectionError(e.to_string())
}

fn query_err(e: reqwest::Error) -> TsdbError {
    TsdbError::QueryError(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use chrono::TimeZone;

    async fn mock(routes: Vec<(&str, axum::routing::MethodRouter)>) -> String {
        let mut app = axum::Router::new();
        for (path, router) in routes {
            app = app.route(path, router);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn make_point(day: u32, tags: &[(&str, &str)]) -> Point {
        let mut point = Point {
            measurement: "cpu".into(),
            tags: Tags::new(),
            fields: Fields::new(),
            timestamp: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).single().unwrap(),
        };
        point.fields.insert("value".into(), serde_json::json!(42.0));
        for (k, v) in tags {
            point.tags.insert(k.to_string(), v.to_string());
        }
        point
    }

    #[tokio::test]
    async fn write_sends_line_protocol() {
        let base = mock(vec![(
            "/api/v2/write",
            post(|req: Request<Body>| async move {
                let body = axum::body::to_bytes(req.into_body(), 4096).await.unwrap();
                let text = String::from_utf8(body.to_vec()).unwrap();
                assert!(text.contains("cpu,host=srv\\ 1 value=42.0 1767225600000000000"));
                (StatusCode::NO_CONTENT, "")
            }),
        )])
        .await;
        let db = InfluxDB::new_with_credentials(base, "bee", "myorg");
        db.write_point(make_point(1, &[("host", "srv 1")])).await.unwrap();
    }

    #[tokio::test]
    async fn query_sends_flux_and_parses_csv() {
        let base = mock(vec![(
            "/api/v2/query",
            post(|req: Request<Body>| async move {
                let body = axum::body::to_bytes(req.into_body(), 4096).await.unwrap();
                let text = String::from_utf8(body.to_vec()).unwrap();
                assert!(text.contains("from(bucket: \\\"bee\\\")"));
                assert!(text.contains("r._measurement == \\\"cpu\\\""));
                assert!(text.contains("r.host == \\\"srv1\\\""));
                (
                    StatusCode::OK,
                    "#datatype,string,long,dateTime:RFC3339,string,string,double\n\
                     #group,false,false,false,true,true,false\n\
                     #default,_result,,,,,\n\
                     ,result,table,_time,_measurement,_field,_value\n\
                     ,,0,2026-01-01T00:00:00Z,cpu,value,42\n",
                )
            }),
        )])
        .await;
        let db = InfluxDB::new(base);
        let filter = TagFilter { key: "host".into(), value: "srv1".into(), op: FilterOp::Eq };
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 31, 0, 0, 0).single().unwrap();
        let points = db.query_range("cpu", start, end, Some(&[filter])).await.unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].measurement, "cpu");
        assert_eq!(points[0].fields["value"], 42.0);
        assert_eq!(points[0].timestamp.timestamp(), 1767225600);
    }

    #[tokio::test]
    async fn create_cq_sends_task() {
        let base = mock(vec![(
            "/api/v2/tasks",
            post(|req: Request<Body>| async move {
                let body = axum::body::to_bytes(req.into_body(), 4096).await.unwrap();
                let text = String::from_utf8(body.to_vec()).unwrap();
                assert!(text.contains("option task"));
                assert!(text.contains("mean()"));
                assert!(text.contains("to(bucket: \\\"downsampled\\\")"));
                (StatusCode::CREATED, axum::Json(serde_json::json!({"id": "1"})))
            }),
        )])
        .await;
        let db = InfluxDB::new(base);
        db.create_continuous_query(CQSpec {
            name: "cq1".into(),
            source: "raw".into(),
            target: "downsampled".into(),
            every: "1m".into(),
            aggregation: Aggregation::Mean,
        })
        .await
        .unwrap();
    }

    #[test]
    fn flux_escaping_prevents_injection() {
        assert_eq!(flux_str("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(flux_regex("a/b"), "a\\/b");
        assert!(flux_key("good_key").is_ok());
        assert!(flux_key("bad key").is_err());
        assert!(flux_duration("1m").is_ok());
        assert!(flux_duration("2h30m").is_err());
    }

    #[tokio::test]
    async fn query_escapes_user_values() {
        let base = mock(vec![(
            "/api/v2/query",
            post(|req: Request<Body>| async move {
                let body = axum::body::to_bytes(req.into_body(), 4096).await.unwrap();
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let flux = v["query"].as_str().unwrap();
                assert!(flux.contains("r.host == \"srv\\\"x\\\"\""), "got: {flux}");
                assert!(!flux.contains("\"srv\"x\""), "raw quote must not appear: {flux}");
                (StatusCode::OK, "")
            }),
        )])
        .await;
        let db = InfluxDB::new(base);
        let filter = TagFilter { key: "host".into(), value: "srv\"x\"".into(), op: FilterOp::Eq };
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).single().unwrap();
        db.query_range("cpu", start, end, Some(&[filter])).await.unwrap();
    }

    #[test]
    fn parse_csv_handles_quoted_values() {
        let body = "\
#datatype,string,long,dateTime:RFC3339,string,string,string\n\
#default,_result,,,,,\n\
,result,table,_time,_measurement,_field,_value\n\
,,0,2026-01-01T00:00:00Z,cpu,note,\"a, b\"\n";
        let points = parse_csv(body).unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].fields["note"], "a, b");
    }

    #[tokio::test]
    async fn query_error_on_non_2xx() {
        let base =
            mock(vec![("/api/v2/query", post(|| async { StatusCode::INTERNAL_SERVER_ERROR }))])
                .await;
        let db = InfluxDB::new(base);
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).single().unwrap();
        let err = db.query_range("cpu", start, end, None).await.unwrap_err();
        assert!(matches!(err, TsdbError::QueryError(_)));
    }
}
