// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! SQL-shape checks for [`QuerySet`]: the implicit soft-delete filter, the
//! parameterised filters (incl. `filter_in`) and the aggregates.

use super::*;
use crate::QuerySet;

#[test]
fn soft_reads_filter_on_the_flag_first() {
    let qs = QuerySet::<Post>::new("posts").filter_eq("published", 1).unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM posts WHERE deleted = ? AND published = ?");
    // `params()` is the user-params accessor; execution binds the flag itself.
    assert_eq!(qs.params(), &[Value::Int(1)]);
}

#[test]
fn with_deleted_drops_the_flag_term() {
    let qs = QuerySet::<Post>::new("posts").with_deleted().limit(1);
    assert_eq!(qs.to_sql(), "SELECT * FROM posts LIMIT 1");
}

#[test]
fn plain_models_keep_their_sql_shape() {
    let qs = QuerySet::<Plain>::new("plain").filter_gt("id", 3).unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM plain WHERE id > ?");
}

#[tokio::test]
async fn soft_delete_sets_the_flag_under_the_active_filter() {
    let db = Mock::default();
    QuerySet::<Post>::new("posts").filter_eq("id", 3).unwrap().delete(&db).await.unwrap();
    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "UPDATE posts SET deleted = ? WHERE deleted = ? AND id = ?");
    assert_eq!(params, vec![Value::Bool(true), Value::Bool(false), Value::Int(3)]);
}

#[tokio::test]
async fn hard_delete_purges_under_the_active_filter() {
    let db = Mock::default();
    QuerySet::<Post>::new("posts").hard_delete(&db).await.unwrap();
    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "DELETE FROM posts WHERE deleted = ?");
    assert_eq!(params, vec![Value::Bool(false)]);

    QuerySet::<Post>::new("posts").with_deleted().hard_delete(&db).await.unwrap();
    let (sql, params) = db.calls().pop().unwrap();
    assert_eq!(sql, "DELETE FROM posts");
    assert!(params.is_empty());
}

#[tokio::test]
async fn update_binds_sets_before_the_flag() {
    let db = Mock::default();
    let qs = QuerySet::<Post>::new("posts").filter_eq("id", 3).unwrap();
    qs.update(&db, &[("title", Value::Text("t".into()))]).await.unwrap();
    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "UPDATE posts SET title = ? WHERE deleted = ? AND id = ?");
    assert_eq!(params, vec![Value::Text("t".into()), Value::Bool(false), Value::Int(3)]);
}

#[test]
fn filter_in_renders_one_placeholder_per_value_in_order() {
    let qs = QuerySet::<Plain>::new("plain")
        .filter_in("id", &[Value::Int(3), Value::Int(1), Value::Int(2)])
        .unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM plain WHERE id IN (?, ?, ?)");
    assert_eq!(qs.params(), &[Value::Int(3), Value::Int(1), Value::Int(2)]);
}

#[test]
fn filter_in_with_an_empty_slice_is_an_error() {
    let err = QuerySet::<Plain>::new("plain").filter_in("id", &[]).err().unwrap();
    assert!(matches!(&err, OrmError::QueryError(message) if message.contains("empty value list")));
}

#[test]
fn filter_in_validates_the_field_name() {
    let err =
        QuerySet::<Plain>::new("plain").filter_in("id) OR 1=1 --", &[Value::Int(1)]).err().unwrap();
    assert!(matches!(err, OrmError::InvalidField(_)));
}

#[tokio::test]
async fn filter_in_binds_after_the_soft_flag() {
    let db = Mock::default();
    QuerySet::<Post>::new("posts")
        .filter_in("id", &[Value::Int(1), Value::Int(2)])
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "SELECT * FROM posts WHERE deleted = ? AND id IN (?, ?)");
    assert_eq!(params, vec![Value::Bool(false), Value::Int(1), Value::Int(2)]);
}

#[tokio::test]
async fn sum_reads_numbers_and_numeric_strings() {
    let db = Mock::with_rows(vec![value_row(serde_json::json!(12.5))]);
    let qs = QuerySet::<Post>::new("posts").filter_gt("price", 0).unwrap();
    assert_eq!(qs.sum(&db, "price").await.unwrap(), Some(12.5));
    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "SELECT SUM(price) AS value FROM posts WHERE deleted = ? AND price > ?");
    assert_eq!(params, vec![Value::Bool(false), Value::Int(0)]);

    // pg returns NUMERIC as a string.
    let db = Mock::with_rows(vec![value_row(serde_json::json!("12.5"))]);
    assert_eq!(QuerySet::<Post>::new("posts").avg(&db, "price").await.unwrap(), Some(12.5));
}

#[tokio::test]
async fn aggregates_on_empty_results_return_none_or_a_shape_error() {
    let db = Mock::with_rows(vec![value_row(serde_json::Value::Null)]);
    assert_eq!(QuerySet::<Post>::new("posts").sum(&db, "price").await.unwrap(), None);

    let db = Mock::default();
    let err = QuerySet::<Post>::new("posts").min(&db, "title").await.unwrap_err();
    assert!(err.to_string().contains("unexpected result shape"));
}

#[tokio::test]
async fn non_numeric_sum_is_an_error_but_min_passes_through() {
    let db = Mock::with_rows(vec![value_row(serde_json::json!("abc"))]);
    let err = QuerySet::<Post>::new("posts").sum(&db, "title").await.unwrap_err();
    assert!(err.to_string().contains("non-numeric"));

    let db = Mock::with_rows(vec![value_row(serde_json::json!("abc"))]);
    let value = QuerySet::<Post>::new("posts").max(&db, "title").await.unwrap();
    assert_eq!(value, Some(serde_json::json!("abc")));
}

#[tokio::test]
async fn invalid_aggregate_field_is_rejected_without_a_query() {
    let db = Mock::default();
    let err = QuerySet::<Post>::new("posts").sum(&db, "price; DROP TABLE posts").await.unwrap_err();
    assert!(matches!(err, OrmError::InvalidField(_)));
    assert!(db.calls().is_empty());
}
