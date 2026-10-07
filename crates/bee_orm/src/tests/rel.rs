// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Behavior checks for [`rel`](crate::rel) against the recording mock: fk
//! discovery, the soft filter, batched `children_for` alignment/chunking and
//! `belongs_to`.

use super::*;
use crate::rel::{self, grouping_key};

const CHILD_SELECT: &str = "SELECT * FROM articles WHERE deleted = ? AND author_id = ?";

fn article_row(id: i64, author_id: i64) -> Row {
    row_of(&[
        ("id", serde_json::json!(id)),
        ("author_id", serde_json::json!(author_id)),
        ("deleted", serde_json::json!(false)),
    ])
}

#[test]
fn fk_column_to_finds_the_declared_reference() {
    assert_eq!(rel::fk_column_to::<Article, Author>(), Some("author_id"));
    assert_eq!(rel::fk_column_to::<Author, Article>(), None);
}

#[tokio::test]
async fn children_query_carries_the_child_soft_filter() {
    let db = Mock::with_rows(vec![article_row(1, 7), article_row(2, 7)]);
    let children = rel::children::<Article, Author, _>(&db, &Author { id: 7 }).await.unwrap();
    assert_eq!(children.iter().map(|a| a.id).collect::<Vec<_>>(), vec![1, 2]);
    assert!(children.iter().all(|a| a.author_id == 7 && !a.deleted));

    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, CHILD_SELECT);
    assert_eq!(params, vec![Value::Bool(false), Value::Int(7)]);
}

#[test]
fn children_without_fk_metadata_names_both_models() {
    let db = Mock::default();
    let err =
        rel::children_query::<Author, Article>(&Article { id: 1, author_id: 2, deleted: false })
            .err()
            .unwrap();
    let message = err.to_string();
    assert!(message.contains("authors") && message.contains("articles"), "got: {message}");
    assert!(message.contains("fk"), "got: {message}");
    assert!(db.calls().is_empty());
}

#[tokio::test]
async fn children_for_is_index_aligned_and_dedupes_the_query() {
    let db = Mock::with_rows(vec![
        article_row(1, 1),
        article_row(2, 2),
        article_row(3, 1),
        article_row(9, 99), // not requested: dropped from every group
    ]);
    let parents = [Author { id: 1 }, Author { id: 2 }, Author { id: 1 }];
    let groups = rel::children_for::<Article, Author, _>(&db, &parents).await.unwrap();
    let ids: Vec<Vec<i64>> =
        groups.iter().map(|group| group.iter().map(|a| a.id).collect()).collect();
    assert_eq!(ids, vec![vec![1, 3], vec![2], vec![1, 3]]);

    // One query, deduped keys in first-seen order, soft flag first.
    let calls = db.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "SELECT * FROM articles WHERE deleted = ? AND author_id IN (?, ?)");
    assert_eq!(calls[0].1, vec![Value::Bool(false), Value::Int(1), Value::Int(2)]);
}

#[tokio::test]
async fn children_for_with_no_parents_queries_nothing() {
    let db = Mock::default();
    assert!(rel::children_for::<Article, Author, _>(&db, &[]).await.unwrap().is_empty());
    assert!(db.calls().is_empty());
}

#[tokio::test]
async fn children_for_chunks_at_the_parameter_ceiling() {
    let db = Mock::default();
    let parents: Vec<Author> = (0..1000).map(|id| Author { id }).collect();
    let groups = rel::children_for::<Article, Author, _>(&db, &parents).await.unwrap();
    assert_eq!(groups.len(), 1000);
    assert!(groups.iter().all(Vec::is_empty));

    let calls = db.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1.len(), 1 + 999); // soft flag + the first chunk
    assert_eq!(calls[1].1.len(), 1 + 1);
}

#[tokio::test]
async fn belongs_to_reads_the_pk_under_the_soft_filter() {
    let db = Mock::with_rows(vec![row_of(&[("id", serde_json::json!(5))])]);
    let parent = rel::belongs_to::<Author, _>(&db, 5).await.unwrap();
    assert_eq!(parent.map(|author| author.id), Some(5));

    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "SELECT * FROM authors WHERE deleted = ? AND id = ? LIMIT 1");
    assert_eq!(params, vec![Value::Bool(false), Value::Int(5)]);
}

#[test]
fn grouping_key_bridges_bool_and_integer_spellings() {
    assert_eq!(grouping_key(&serde_json::json!(true)), "1");
    assert_eq!(grouping_key(&serde_json::json!(1)), "1");
    assert_eq!(grouping_key(&serde_json::json!(false)), "0");
    assert_eq!(grouping_key(&serde_json::json!(0)), "0");
    // Text stays distinct from the integer spelling.
    assert_eq!(grouping_key(&serde_json::json!("1")), "\"1\"");
    assert_eq!(grouping_key(&serde_json::Value::Null), "null");
}

/// `json_of` must hand back the same serde form the read path produces, so a
/// bound value and a decoded cell land on one grouping key.
#[cfg(feature = "chrono")]
#[test]
fn json_of_chrono_values_matches_the_read_cell() {
    use chrono::{DateTime, NaiveDate, Utc};
    let date = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
    let naive = date.and_hms_micro_opt(12, 34, 56, 123456).unwrap();
    let utc = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);

    assert_eq!(rel::json_of(&Value::from(date)), serde_json::json!("2026-10-06"));
    assert_eq!(rel::json_of(&Value::from(naive)), serde_json::json!("2026-10-06T12:34:56.123456"));
    assert_eq!(rel::json_of(&Value::from(utc)), serde_json::json!("2026-10-06T12:34:56.123456Z"));
}

#[cfg(feature = "rust_decimal")]
#[test]
fn json_of_decimal_matches_the_read_cell() {
    let decimal: rust_decimal::Decimal = "1.50".parse().unwrap();
    assert_eq!(rel::json_of(&Value::from(decimal)), serde_json::json!("1.50"));
}
