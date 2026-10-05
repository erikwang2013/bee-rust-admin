// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use async_trait::async_trait;

use crate::rest::RestEngine;
use crate::{
    AggResult, Aggregations, BulkResult, Document, DocumentId, Mapping, ScrollHandle, SearchEngine,
    SearchError, SearchQuery, SearchResult,
};

/// OpenSearch driver backed by its REST API (compatible with the
/// Elasticsearch 7.x API surface used here).
pub struct OpenSearch(RestEngine);

impl OpenSearch {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self(RestEngine::new(base_url, "opensearch"))
    }
}

#[async_trait]
impl SearchEngine for OpenSearch {
    async fn create_index(&self, name: &str, mapping: Option<Mapping>) -> Result<(), SearchError> {
        self.0.create_index(name, mapping).await
    }

    async fn delete_index(&self, name: &str) -> Result<(), SearchError> {
        self.0.delete_index(name).await
    }

    async fn index(&self, index: &str, id: DocumentId, doc: Document) -> Result<(), SearchError> {
        self.0.index(index, id, doc).await
    }

    async fn bulk_index(
        &self,
        index: &str,
        docs: &[(DocumentId, Document)],
    ) -> Result<BulkResult, SearchError> {
        self.0.bulk_index(index, docs).await
    }

    async fn get(&self, index: &str, id: &DocumentId) -> Result<Option<Document>, SearchError> {
        self.0.get(index, id).await
    }

    async fn delete(&self, index: &str, id: &DocumentId) -> Result<(), SearchError> {
        self.0.delete(index, id).await
    }

    async fn search(&self, index: &str, query: SearchQuery) -> Result<SearchResult, SearchError> {
        self.0.search(index, query).await
    }

    async fn scroll(&self, handle: ScrollHandle) -> Result<SearchResult, SearchError> {
        self.0.scroll(handle).await
    }

    async fn aggregate(&self, index: &str, aggs: Aggregations) -> Result<AggResult, SearchError> {
        self.0.aggregate(index, aggs).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::{get, post};

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

    #[tokio::test]
    async fn get_returns_document() {
        let base = mock(vec![(
            "/posts/_doc/1",
            get(|| async {
                (StatusCode::OK, axum::Json(serde_json::json!({"_source": {"title": "hello"}})))
            }),
        )])
        .await;
        let engine = OpenSearch::new(base);
        let doc = engine.get("posts", &"1".into()).await.unwrap().unwrap();
        assert_eq!(doc["title"], "hello");
    }

    #[tokio::test]
    async fn search_parses_hits() {
        let payload = serde_json::json!({
            "hits": {
                "total": { "value": 2 },
                "hits": [
                    { "_id": "a", "_score": 1.0, "_source": {"t": 1} },
                    { "_id": "b", "_score": 0.5, "_source": {"t": 2} }
                ]
            }
        });
        let base = mock(vec![(
            "/posts/_search",
            post(|| async { (StatusCode::OK, axum::Json(payload)) }),
        )])
        .await;
        let engine = OpenSearch::new(base);
        let res = engine.search("posts", serde_json::json!({"match_all": {}})).await.unwrap();
        assert_eq!(res.total, 2);
        assert_eq!(res.hits.len(), 2);
    }

    #[tokio::test]
    async fn bulk_index_reports_partial_failures() {
        let base = mock(vec![(
            "/_bulk",
            post(|| async {
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "errors": true,
                        "items": [
                            {"index": {"status": 201}},
                            {"index": {"status": 400, "error": {"reason": "boom"}}}
                        ]
                    })),
                )
            }),
        )])
        .await;
        let engine = OpenSearch::new(base);
        let res = engine
            .bulk_index(
                "posts",
                &[
                    ("1".into(), serde_json::json!({"a": 1})),
                    ("2".into(), serde_json::json!({"a": 2})),
                ],
            )
            .await
            .unwrap();
        assert_eq!(res.indexed, 1);
        assert_eq!(res.errors.len(), 1);
    }

    #[tokio::test]
    async fn bulk_index_counts_items() {
        let base = mock(vec![(
            "/_bulk",
            post(|req: Request<Body>| async move {
                let body = axum::body::to_bytes(req.into_body(), 4096).await.unwrap();
                assert_eq!(String::from_utf8(body.to_vec()).unwrap().lines().count(), 2);
                (StatusCode::OK, axum::Json(serde_json::json!({"items": [{}]})))
            }),
        )])
        .await;
        let engine = OpenSearch::new(base);
        let res =
            engine.bulk_index("posts", &[("1".into(), serde_json::json!({"a": 1}))]).await.unwrap();
        assert_eq!(res.indexed, 1);
    }
}
