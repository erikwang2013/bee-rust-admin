// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! In-crate unit tests: a recording [`Db`] mock, shared model fixtures, and
//! SQL-shape checks per area — [`Model`](crate::Model) operations
//! ([`model_ops`]), [`QuerySet`] ([`queryset`]), [`migrate`](crate::migrate)
//! and [`rel`](crate::rel). Behavior against real backends lives in
//! `tests/**`.

mod datetime_types;
mod m2m;
mod migrate;
mod model_ops;
mod queryset;
mod rel;

use std::sync::Mutex;

use async_trait::async_trait;

use crate::{
    ColumnDef, Db, DefaultValue, Dialect, M2mDef, Model, OrmError, Reference, Result, Row, SqlType,
    Value,
};

/// Records every statement; `query` answers with the preset rows, `execute`
/// reports one affected row per call.
#[derive(Default)]
struct Mock {
    rows: Vec<Row>,
    dialect: Option<Dialect>,
    calls: Mutex<Vec<(String, Vec<Value>)>>,
}

impl Mock {
    fn with_rows(rows: Vec<Row>) -> Self {
        Self { rows, ..Self::default() }
    }

    fn with_dialect(dialect: Dialect) -> Self {
        Self { dialect: Some(dialect), ..Self::default() }
    }

    fn calls(&self) -> Vec<(String, Vec<Value>)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Db for Mock {
    fn dialect(&self) -> Option<Dialect> {
        self.dialect
    }

    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        self.calls.lock().unwrap().push((sql.to_string(), params.to_vec()));
        Ok(self.rows.clone())
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        self.calls.lock().unwrap().push((sql.to_string(), params.to_vec()));
        Ok(1)
    }
}

fn value_row(value: serde_json::Value) -> Row {
    let mut row = Row::new();
    row.insert("value".into(), value);
    row
}

/// A result row from `(column, value)` pairs (relation result sets).
fn row_of(columns: &[(&str, serde_json::Value)]) -> Row {
    columns.iter().map(|(name, value)| ((*name).to_string(), value.clone())).collect()
}

/// One writable integer column, no timestamps: the plain-SQL control.
struct Tiny {
    id: i64,
    v: i64,
}

impl Model for Tiny {
    fn table_name() -> &'static str {
        "tiny_items"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(_row: &Row) -> Result<Self> {
        Err(OrmError::QueryError("unused in these tests".into()))
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("v", Value::Int(self.v))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![("v", Value::Int(self.v))]
    }
}

/// Timestamps + a soft-delete flag + a plain column.
struct Note {
    id: i64,
    title: String,
    deleted: bool,
}

impl Model for Note {
    fn table_name() -> &'static str {
        "notes"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(_row: &Row) -> Result<Self> {
        Err(OrmError::QueryError("unused in these tests".into()))
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("title", Value::Text(self.title.clone())), ("deleted", Value::Bool(self.deleted))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![("title", Value::Text(self.title.clone()))]
    }
    fn auto_now_add_columns() -> &'static [&'static str] {
        &["created"]
    }
    fn auto_now_columns() -> &'static [&'static str] {
        &["updated"]
    }
    fn soft_delete_column() -> Option<&'static str> {
        Some("deleted")
    }
}

fn note(id: i64) -> Note {
    Note { id, title: "n".into(), deleted: false }
}

/// Hooks that fail for the titles "blocked" (before) and "late" (after).
#[derive(Debug)]
struct Guarded {
    id: i64,
    title: String,
}

#[async_trait]
impl Model for Guarded {
    fn table_name() -> &'static str {
        "guarded"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(_row: &Row) -> Result<Self> {
        Err(OrmError::QueryError("unused in these tests".into()))
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("title", Value::Text(self.title.clone()))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![("title", Value::Text(self.title.clone()))]
    }
    async fn before_insert(&self) -> Result<()> {
        if self.title == "blocked" {
            Err(OrmError::QueryError("blocked by before_insert".into()))
        } else {
            Ok(())
        }
    }
    async fn after_insert(&self) -> Result<()> {
        if self.title == "late" {
            Err(OrmError::QueryError("raised by after_insert".into()))
        } else {
            Ok(())
        }
    }
}

/// Soft-delete model for the QuerySet tests (no timestamps).
struct Post {
    id: i64,
}

impl Model for Post {
    fn table_name() -> &'static str {
        "posts"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(_row: &Row) -> Result<Self> {
        Ok(Self { id: 0 })
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("id", Value::Int(self.id))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![]
    }
    fn soft_delete_column() -> Option<&'static str> {
        Some("deleted")
    }
}

/// Same shape without a soft-delete column.
struct Plain {
    id: i64,
}

impl Model for Plain {
    fn table_name() -> &'static str {
        "plain"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(_row: &Row) -> Result<Self> {
        Ok(Self { id: 0 })
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("id", Value::Int(self.id))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![]
    }
}

/// Parent fixture for the relation tests, with schema metadata (auto pk, a
/// soft-delete flag) — the shape a derive would emit.
struct Author {
    id: i64,
}

impl Model for Author {
    fn table_name() -> &'static str {
        "authors"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self { id: row.get("id").and_then(serde_json::Value::as_i64).unwrap_or_default() })
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("id", Value::Int(self.id))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![]
    }
    fn soft_delete_column() -> Option<&'static str> {
        Some("deleted")
    }
    fn columns() -> &'static [ColumnDef] {
        &[
            ColumnDef {
                name: "id",
                sql: SqlType::BigInt,
                nullable: false,
                primary_key: true,
                auto_increment: true,
                default: None,
                references: None,
            },
            ColumnDef {
                name: "deleted",
                sql: SqlType::Bool,
                nullable: false,
                primary_key: false,
                auto_increment: false,
                default: Some(DefaultValue::Bool(false)),
                references: None,
            },
        ]
    }
}

/// Child fixture: an fk to [`Author`], a soft-delete flag and a timestamp
/// column — the metadata trio the migration tests assert on.
struct Article {
    id: i64,
    author_id: i64,
    deleted: bool,
}

impl Model for Article {
    fn table_name() -> &'static str {
        "articles"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.get("id").and_then(serde_json::Value::as_i64).unwrap_or_default(),
            author_id: row.get("author_id").and_then(serde_json::Value::as_i64).unwrap_or_default(),
            deleted: row.get("deleted").and_then(serde_json::Value::as_bool).unwrap_or(false),
        })
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("author_id", Value::Int(self.author_id)), ("deleted", Value::Bool(self.deleted))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![("author_id", Value::Int(self.author_id))]
    }
    fn soft_delete_column() -> Option<&'static str> {
        Some("deleted")
    }
    fn columns() -> &'static [ColumnDef] {
        &[
            ColumnDef {
                name: "id",
                sql: SqlType::BigInt,
                nullable: false,
                primary_key: true,
                auto_increment: true,
                default: None,
                references: None,
            },
            ColumnDef {
                name: "author_id",
                sql: SqlType::BigInt,
                nullable: false,
                primary_key: false,
                auto_increment: false,
                default: None,
                references: Some(Reference {
                    table: Author::table_name,
                    pk_column: Author::pk_column,
                }),
            },
            ColumnDef {
                name: "deleted",
                sql: SqlType::Bool,
                nullable: false,
                primary_key: false,
                auto_increment: false,
                default: Some(DefaultValue::Bool(false)),
                references: None,
            },
            ColumnDef {
                name: "created",
                sql: SqlType::BigInt,
                nullable: false,
                primary_key: false,
                auto_increment: false,
                default: Some(DefaultValue::Int(0)),
                references: None,
            },
        ]
    }
}

/// Declaring model for the m2m tests: one relation to [`Author`] (both sides
/// carry `columns()`, so the join DDL has real types) — the shape a derive
/// would emit.
struct Reader {
    id: i64,
}

impl Model for Reader {
    fn table_name() -> &'static str {
        "readers"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(_row: &Row) -> Result<Self> {
        Err(OrmError::QueryError("unused in these tests".into()))
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("id", Value::Int(self.id))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![]
    }
    fn columns() -> &'static [ColumnDef] {
        &[
            ColumnDef {
                name: "id",
                sql: SqlType::BigInt,
                nullable: false,
                primary_key: true,
                auto_increment: true,
                default: None,
                references: None,
            },
            ColumnDef {
                name: "author_id",
                sql: SqlType::BigInt,
                nullable: false,
                primary_key: false,
                auto_increment: false,
                default: None,
                references: Some(Reference {
                    table: Author::table_name,
                    pk_column: Author::pk_column,
                }),
            },
        ]
    }
    fn m2m() -> &'static [M2mDef] {
        &[M2mDef {
            table: "reader_author",
            local_column: "reader_id",
            foreign_column: "author_id",
            target_table: Author::table_name,
            target_columns: Author::columns,
            target_ident: "Author",
        }]
    }
}

/// Text pk (non-auto) + a boolean default: the non-auto pk / default edge.
struct Setting {
    name: String,
    enabled: bool,
}

impl Model for Setting {
    fn table_name() -> &'static str {
        "settings"
    }
    fn pk_column() -> &'static str {
        "name"
    }
    fn from_row(_row: &Row) -> Result<Self> {
        Err(OrmError::QueryError("unused in these tests".into()))
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("name", Value::Text(self.name.clone())), ("enabled", Value::Bool(self.enabled))]
    }
    fn pk_value(&self) -> Value {
        Value::Text(self.name.clone())
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        vec![("enabled", Value::Bool(self.enabled))]
    }
    fn columns() -> &'static [ColumnDef] {
        &[
            ColumnDef {
                name: "name",
                sql: SqlType::Text,
                nullable: false,
                primary_key: true,
                auto_increment: false,
                default: None,
                references: None,
            },
            ColumnDef {
                name: "enabled",
                sql: SqlType::Bool,
                nullable: false,
                primary_key: false,
                auto_increment: false,
                default: Some(DefaultValue::Bool(true)),
                references: None,
            },
        ]
    }
}
