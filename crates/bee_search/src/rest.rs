// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use async_trait::async_trait;
use reqwest::{Client, StatusCode};

use crate::{
    AggResult, Aggregations, BulkResult, Document, DocumentId, Mapping, ScrollHandle, SearchEngine,
    SearchError, SearchHit, SearchQuery, SearchResult, http_client,
};

/// Shared REST implementation for Elasticsearch-compatible engines
/// (Elasticsearch, OpenSearch), parameterized by the driver name used in
/// error messages so callers can tell backends apart.
pub(crate) struct RestEngine {
    client: Client,
    base_url: String,
    name: &'static str,
}

impl RestEngine {
    pub(crate) fn new(base_url: impl Into<String>, name: &'static str) -> Self {
        Self { client: http_client(), base_url: base_url.into(), name }
    }
}

#[async_trait]
impl SearchEngine for RestEngine {
    async fn create_index(&self, name: &str, mapping: Option<Mapping>) -> Result<(), SearchError> {
        let res = self
            .client
            .put(endpoint(&self.base_url, &[name])?)
            .json(&mapping.unwrap_or_default())
            .send()
            .await
            .map_err(conn_err)?;
        check_status(res, "create_index").await
    }

    async fn delete_index(&self, name: &str) -> Result<(), SearchError> {
        let res = self
            .client
            .delete(endpoint(&self.base_url, &[name])?)
            .send()
            .await
            .map_err(conn_err)?;
        check_status(res, "delete_index").await
    }

    async fn index(&self, index: &str, id: DocumentId, doc: Document) -> Result<(), SearchError> {
        let res = self
            .client
            .put(endpoint(&self.base_url, &[index, "_doc", id.as_str()])?)
            .json(&doc)
            .send()
            .await
            .map_err(conn_err)?;
        check_status(res, "index").await
    }

    async fn bulk_index(
        &self,
        index: &str,
        docs: &[(DocumentId, Document)],
    ) -> Result<BulkResult, SearchError> {
        // NDJSON: one action line + one document line per item. Action lines
        // are JSON-serialized so ids are escaped — a raw `"` would break out
        // of the action object and inject arbitrary bulk operations.
        let mut body = String::new();
        for (id, doc) in docs {
            let action = serde_json::json!({ "index": { "_index": index, "_id": id } });
            body.push_str(&action.to_string());
            body.push('\n');
            body.push_str(&doc.to_string());
            body.push('\n');
        }
        let res = self
            .client
            .post(endpoint(&self.base_url, &["_bulk"])?)
            .header("content-type", "application/x-ndjson")
            .body(body)
            .send()
            .await
            .map_err(conn_err)?;
        if !res.status().is_success() {
            return Err(http_error(res, "bulk_index", self.name).await);
        }
        let payload: serde_json::Value = res.json().await.map_err(query_err)?;
        // `_bulk` returns 200 even when items fail (`errors: true`); count
        // only successful items and surface the per-item errors.
        let mut indexed = 0u64;
        let mut errors = Vec::new();
        if let Some(items) = payload["items"].as_array() {
            for item in items {
                match item.get("index").and_then(|i| i.get("error")) {
                    Some(err) => errors.push(err.to_string()),
                    None => indexed += 1,
                }
            }
        }
        Ok(BulkResult { indexed, errors })
    }

    async fn get(&self, index: &str, id: &DocumentId) -> Result<Option<Document>, SearchError> {
        let res = self
            .client
            .get(endpoint(&self.base_url, &[index, "_doc", id.as_str()])?)
            .send()
            .await
            .map_err(conn_err)?;
        if res.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !res.status().is_success() {
            return Err(http_error(res, "get", self.name).await);
        }
        let payload: serde_json::Value = res.json().await.map_err(query_err)?;
        Ok(payload.get("_source").cloned().filter(|v| !v.is_null()))
    }

    async fn delete(&self, index: &str, id: &DocumentId) -> Result<(), SearchError> {
        let res = self
            .client
            .delete(endpoint(&self.base_url, &[index, "_doc", id.as_str()])?)
            .send()
            .await
            .map_err(conn_err)?;
        check_status(res, "delete").await
    }

    async fn search(&self, index: &str, query: SearchQuery) -> Result<SearchResult, SearchError> {
        let res = self
            .client
            .post(endpoint(&self.base_url, &[index, "_search"])?)
            .json(&query)
            .send()
            .await
            .map_err(conn_err)?;
        if !res.status().is_success() {
            return Err(http_error(res, "search", self.name).await);
        }
        let payload: serde_json::Value = res.json().await.map_err(query_err)?;
        Ok(parse_hits(&payload))
    }

    async fn scroll(&self, handle: ScrollHandle) -> Result<SearchResult, SearchError> {
        let res = self
            .client
            .post(endpoint(&self.base_url, &["_search", "scroll"])?)
            .json(&serde_json::json!({ "scroll_id": handle }))
            .send()
            .await
            .map_err(conn_err)?;
        if !res.status().is_success() {
            return Err(http_error(res, "scroll", self.name).await);
        }
        let payload: serde_json::Value = res.json().await.map_err(query_err)?;
        Ok(parse_hits(&payload))
    }

    async fn aggregate(&self, index: &str, aggs: Aggregations) -> Result<AggResult, SearchError> {
        let query = serde_json::json!({ "size": 0, "aggs": aggs });
        let res = self
            .client
            .post(endpoint(&self.base_url, &[index, "_search"])?)
            .json(&query)
            .send()
            .await
            .map_err(conn_err)?;
        if !res.status().is_success() {
            return Err(http_error(res, "aggregate", self.name).await);
        }
        let payload: serde_json::Value = res.json().await.map_err(query_err)?;
        Ok(payload.get("aggregations").cloned().unwrap_or_default())
    }
}

/// Builds `{base}/{segments...}` with each segment percent-encoded, so ids
/// or index names cannot escape the URL path. Dot segments are rejected
/// outright: `..` survives percent-encoding and would normalize away to a
/// sibling path on re-parse, rewriting the request to an arbitrary endpoint.
fn endpoint(base: &str, segments: &[&str]) -> Result<String, SearchError> {
    for s in segments {
        if s.is_empty() || s.split('/').any(|p| p == "." || p == "..") {
            return Err(SearchError::IndexError(format!("invalid url segment: {s:?}")));
        }
    }
    let mut url = reqwest::Url::parse(base)
        .map_err(|e| SearchError::ConnectionError(format!("invalid base url {base:?}: {e}")))?;
    url.path_segments_mut()
        .map_err(|_| SearchError::ConnectionError("base url is opaque".into()))?
        .extend(segments);
    Ok(url.to_string())
}

fn parse_hits(payload: &serde_json::Value) -> SearchResult {
    let total = payload["hits"]["total"]["value"].as_u64().unwrap_or(0);
    let hits: Vec<SearchHit> = payload["hits"]["hits"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|h| SearchHit {
                    id: h["_id"].as_str().unwrap_or_default().to_string(),
                    score: h["_score"].as_f64().unwrap_or(0.0),
                    source: h["_source"].clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let aggregations = payload.get("aggregations").cloned();
    SearchResult { total, hits, aggregations }
}

async fn check_status(res: reqwest::Response, op: &str) -> Result<(), SearchError> {
    if res.status().is_success() { Ok(()) } else { Err(http_error(res, op, "").await) }
}

async fn http_error(res: reqwest::Response, op: &str, name: &str) -> SearchError {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    SearchError::QueryError(format!("{name} {op} failed with {status}: {body}"))
}

fn conn_err(e: reqwest::Error) -> SearchError {
    SearchError::ConnectionError(e.to_string())
}

fn query_err(e: reqwest::Error) -> SearchError {
    SearchError::QueryError(e.to_string())
}
