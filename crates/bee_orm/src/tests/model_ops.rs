// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! SQL-shape checks for the default [`Model`](crate::Model) operations:
//! timestamps, soft delete, hooks, `insert_many`.

use super::*;

#[tokio::test]
async fn insert_without_timestamps_stays_plain() {
    let db = Mock::default();
    assert_eq!(Tiny { id: 1, v: 5 }.insert(&db).await.unwrap(), 1);
    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "INSERT INTO tiny_items (v) VALUES (?)");
    assert_eq!(params, vec![Value::Int(5)]);
}

#[tokio::test]
async fn insert_injects_one_now_after_the_own_values() {
    let db = Mock::default();
    note(1).insert(&db).await.unwrap();
    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "INSERT INTO notes (title, deleted, created, updated) VALUES (?, ?, ?, ?)");
    assert_eq!(params[0], Value::Text("n".into()));
    assert_eq!(params[1], Value::Bool(false));
    match (&params[2], &params[3]) {
        (Value::Int(created), Value::Int(updated)) => {
            assert!(*created > 0);
            assert_eq!(created, updated);
        }
        other => panic!("expected two timestamps, got {other:?}"),
    }
}

#[tokio::test]
async fn update_refreshes_auto_now_only() {
    let db = Mock::default();
    note(7).update(&db).await.unwrap();
    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "UPDATE notes SET title = ?, updated = ? WHERE id = ?");
    assert_eq!(params[0], Value::Text("n".into()));
    assert!(matches!(&params[1], Value::Int(now) if *now > 0));
    assert_eq!(params[2], Value::Int(7));
}

#[tokio::test]
async fn soft_delete_flips_the_flag_and_guards_on_it() {
    let db = Mock::default();
    note(7).delete(&db).await.unwrap();
    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "UPDATE notes SET deleted = ? WHERE id = ? AND deleted = ?");
    assert_eq!(params, vec![Value::Bool(true), Value::Int(7), Value::Bool(false)]);
}

#[tokio::test]
async fn hard_delete_is_a_plain_delete() {
    let db = Mock::default();
    note(7).hard_delete(&db).await.unwrap();
    let (sql, params) = db.calls().remove(0);
    assert_eq!(sql, "DELETE FROM notes WHERE id = ?");
    assert_eq!(params, vec![Value::Int(7)]);
}

#[tokio::test]
async fn before_hook_error_aborts_before_any_sql() {
    let db = Mock::default();
    let err = Guarded { id: 1, title: "blocked".into() }.insert(&db).await.unwrap_err();
    assert!(err.to_string().contains("blocked by before_insert"));
    assert!(db.calls().is_empty());
}

#[tokio::test]
async fn after_hook_error_propagates_after_the_write() {
    let db = Mock::default();
    let err = Guarded { id: 1, title: "late".into() }.insert(&db).await.unwrap_err();
    assert!(err.to_string().contains("raised by after_insert"));
    assert_eq!(db.calls().len(), 1);
}

#[tokio::test]
async fn insert_many_chunks_at_the_parameter_ceiling() {
    let db = Mock::default();
    let rows: Vec<Tiny> = (0..1000).map(|v| Tiny { id: v, v }).collect();
    assert_eq!(Tiny::insert_many(&db, &rows).await.unwrap(), 2);
    let calls = db.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1.len(), 999);
    assert_eq!(calls[1].1.len(), 1);
    assert!(calls[0].0.starts_with("INSERT INTO tiny_items (v) VALUES (?)"));
}

#[tokio::test]
async fn insert_many_of_nothing_does_nothing() {
    let db = Mock::default();
    assert_eq!(Tiny::insert_many(&db, &[]).await.unwrap(), 0);
    assert!(db.calls().is_empty());
}

/// A model whose pk is assigned by the database (`insert_values` omits it)
/// and whose `from_row` decodes: the `create` read-back fixture.
struct Tick {
    id: i64,
    name: String,
    tag: String,
}

impl Model for Tick {
    fn table_name() -> &'static str {
        "ticks"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0),
            name: row.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            tag: row.get("tag").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        })
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("name", Value::Text(self.name.clone()))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![("name", Value::Text(self.name.clone()))]
    }
}

/// As [`Tick`], but the client assigns the pk: it is part of the insert.
struct Mark {
    id: i64,
    name: String,
}

impl Model for Mark {
    fn table_name() -> &'static str {
        "marks"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0),
            name: row.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        })
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("id", Value::Int(self.id)), ("name", Value::Text(self.name.clone()))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![("name", Value::Text(self.name.clone()))]
    }
}

#[tokio::test]
async fn create_inserts_then_selects_by_pk_without_returning_support() {
    let db = Mock::with_rows(vec![row_of(&[
        ("id", serde_json::json!(9)),
        ("name", serde_json::json!("a")),
        ("tag", serde_json::json!("d")),
    ])]);
    let saved = Tick { id: 0, name: "a".into(), tag: String::new() }.create(&db).await.unwrap();
    assert_eq!((saved.id, saved.name.as_str(), saved.tag.as_str()), (9, "a", "d"));
    let calls = db.calls();
    assert_eq!(calls[0].0, "INSERT INTO ticks (name) VALUES (?)");
    assert_eq!(calls[0].1, vec![Value::Text("a".into())]);
    // The mock's default `insert_returning` ran the insert, then answered
    // `None` — `create` falls back to the pk select.
    assert_eq!(calls[1].0, "SELECT * FROM ticks WHERE id = ?");
    assert_eq!(calls[1].1, vec![Value::Int(0)]);
}

#[tokio::test]
async fn create_with_a_client_pk_skips_the_returning_step() {
    let db = Mock::with_rows(vec![row_of(&[
        ("id", serde_json::json!(7)),
        ("name", serde_json::json!("x")),
    ])]);
    let saved = Mark { id: 7, name: "x".into() }.create(&db).await.unwrap();
    assert_eq!((saved.id, saved.name.as_str()), (7, "x"));
    let calls = db.calls();
    assert_eq!(calls[0].0, "INSERT INTO marks (id, name) VALUES (?, ?)");
    assert_eq!(calls[0].1, vec![Value::Int(7), Value::Text("x".into())]);
    assert_eq!(calls[1].0, "SELECT * FROM marks WHERE id = ?");
    assert_eq!(calls[1].1, vec![Value::Int(7)]);
}

#[tokio::test]
async fn create_runs_before_insert_before_any_sql() {
    let db = Mock::default();
    let err = Guarded { id: 1, title: "blocked".into() }.create(&db).await.unwrap_err();
    assert!(err.to_string().contains("blocked by before_insert"));
    assert!(db.calls().is_empty());
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn create_reads_the_assigned_pk_and_column_defaults_back() {
    use crate::pool::sqlite::Pool;
    let pool = Pool::connect(":memory:", 1).unwrap();
    pool.execute(
        "CREATE TABLE ticks (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         name TEXT NOT NULL, tag TEXT NOT NULL DEFAULT 'd')",
        &[],
    )
    .await
    .unwrap();
    let saved = Tick { id: 0, name: "a".into(), tag: String::new() }.create(&pool).await.unwrap();
    assert_eq!((saved.id, saved.name.as_str(), saved.tag.as_str()), (1, "a", "d"));
    let again = Tick { id: 0, name: "b".into(), tag: String::new() }.create(&pool).await.unwrap();
    assert_eq!(again.id, 2);
}
