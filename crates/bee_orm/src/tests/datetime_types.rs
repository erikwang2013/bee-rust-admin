// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! sqlite storage pins for the date / time / decimal column types (§59#4):
//! binds write ISO-8601 / plain-decimal text and the DDL declares `TEXT` —
//! a `DECIMAL` declaration would take NUMERIC affinity and silently turn the
//! stored text into a `REAL`, which the affinity test below demonstrates.

#![cfg(all(feature = "sqlite", any(feature = "chrono", feature = "rust_decimal")))]

use serde_json::json;

use crate::pool::sqlite::Pool;
use crate::{ColumnDef, Model, OrmError, Row, SqlType, Value, migrate};

/// One column per new sqlite-mapped type — hand-written, the shape a derive
/// would emit for the same struct. The type columns are nullable so each
/// feature's test inserts only its own subset.
struct Dated {
    id: i64,
}

impl Model for Dated {
    fn table_name() -> &'static str {
        "dated"
    }
    fn pk_column() -> &'static str {
        "id"
    }
    fn from_row(_row: &Row) -> crate::Result<Self> {
        Err(OrmError::QueryError("unused in these tests".into()))
    }
    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        vec![("id", Value::Int(self.id))]
    }
    fn pk_value(&self) -> Value {
        Value::Int(self.id)
    }
    fn update_values(&self) -> Vec<(&'static str, Value)> {
        Vec::new()
    }
    fn columns() -> &'static [ColumnDef] {
        &[
            ColumnDef {
                name: "id",
                sql: SqlType::Int,
                nullable: false,
                primary_key: true,
                auto_increment: true,
                default: None,
                references: None,
            },
            #[cfg(feature = "chrono")]
            ColumnDef {
                name: "d",
                sql: SqlType::Date,
                nullable: true,
                primary_key: false,
                auto_increment: false,
                default: None,
                references: None,
            },
            #[cfg(feature = "chrono")]
            ColumnDef {
                name: "dt",
                sql: SqlType::DateTime,
                nullable: true,
                primary_key: false,
                auto_increment: false,
                default: None,
                references: None,
            },
            #[cfg(feature = "chrono")]
            ColumnDef {
                name: "dtz",
                sql: SqlType::DateTimeTz,
                nullable: true,
                primary_key: false,
                auto_increment: false,
                default: None,
                references: None,
            },
            #[cfg(feature = "rust_decimal")]
            ColumnDef {
                name: "dec",
                sql: SqlType::Decimal,
                nullable: true,
                primary_key: false,
                auto_increment: false,
                default: None,
                references: None,
            },
        ]
    }
}

/// The rendered DDL for `dated` on sqlite, straight out of `sqlite_master`.
async fn ddl(pool: &Pool) -> String {
    let rows =
        pool.query("SELECT sql AS sql FROM sqlite_master WHERE name = 'dated'", &[]).await.unwrap();
    rows[0]["sql"].as_str().unwrap().to_string()
}

#[cfg(feature = "chrono")]
#[tokio::test]
async fn chrono_columns_store_as_text_and_are_declared_text() {
    use chrono::{DateTime, NaiveDate, Utc};
    let date = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
    let naive = date.and_hms_micro_opt(12, 34, 56, 123456).unwrap();
    let utc = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);

    let pool = Pool::connect(":memory:", 1).unwrap();
    migrate::create_table::<Dated, _>(&pool).await.unwrap();
    let ddl = ddl(&pool).await;
    for column in ["d TEXT", "dt TEXT", "dtz TEXT"] {
        assert!(ddl.contains(column), "missing `{column}`: {ddl}");
    }

    pool.execute(
        "INSERT INTO dated (id, d, dt, dtz) VALUES (?, ?, ?, ?)",
        &[Value::Int(1), Value::from(date), Value::from(naive), Value::from(utc)],
    )
    .await
    .unwrap();

    // The stored form is the pinned text, and the affinity is text.
    let rows = pool
        .query(
            "SELECT typeof(d) AS td, typeof(dt) AS tdt, typeof(dtz) AS tdtz, \
             d AS d, dt AS dt, dtz AS dtz FROM dated",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows[0]["td"], json!("text"));
    assert_eq!(rows[0]["tdt"], json!("text"));
    assert_eq!(rows[0]["tdtz"], json!("text"));
    assert_eq!(rows[0]["d"], json!("2026-10-06"));
    assert_eq!(rows[0]["dt"], json!("2026-10-06T12:34:56.123456"));
    assert_eq!(rows[0]["dtz"], json!("2026-10-06T12:34:56.123456Z"));
}

#[cfg(feature = "rust_decimal")]
#[tokio::test]
async fn decimal_columns_store_as_text_and_are_declared_text() {
    let decimal: rust_decimal::Decimal = "1.50".parse().unwrap();
    let pool = Pool::connect(":memory:", 1).unwrap();
    migrate::create_table::<Dated, _>(&pool).await.unwrap();
    let ddl = ddl(&pool).await;
    assert!(ddl.contains("dec TEXT"), "{ddl}");
    assert!(!ddl.contains("DECIMAL"), "no NUMERIC affinity: {ddl}");

    pool.execute(
        "INSERT INTO dated (id, dec) VALUES (?, ?)",
        &[Value::Int(1), Value::from(decimal)],
    )
    .await
    .unwrap();
    let rows = pool.query("SELECT typeof(dec) AS t, dec AS dec FROM dated", &[]).await.unwrap();
    assert_eq!(rows[0]["t"], json!("text"));
    assert_eq!(rows[0]["dec"], json!("1.50"));
}

#[cfg(feature = "rust_decimal")]
#[tokio::test]
async fn decimal_text_survives_only_under_text_affinity() {
    // The reason the DDL above must say TEXT: under DECIMAL's NUMERIC affinity
    // the same bound text lands as a REAL, and "1.50" becomes 1.5.
    let decimal: rust_decimal::Decimal = "1.50".parse().unwrap();
    let pool = Pool::connect(":memory:", 1).unwrap();
    pool.execute("CREATE TABLE text_aff (v TEXT)", &[]).await.unwrap();
    pool.execute("CREATE TABLE num_aff (v DECIMAL)", &[]).await.unwrap();
    for table in ["text_aff", "num_aff"] {
        pool.execute(&format!("INSERT INTO {table} (v) VALUES (?)"), &[Value::from(decimal)])
            .await
            .unwrap();
    }
    let rows = pool
        .query(
            "SELECT (SELECT typeof(v) FROM text_aff) AS text_type, \
             (SELECT v FROM text_aff) AS text_value, \
             (SELECT typeof(v) FROM num_aff) AS num_type",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows[0]["text_type"], json!("text"));
    assert_eq!(rows[0]["text_value"], json!("1.50"));
    assert_eq!(rows[0]["num_type"], json!("real"));
}
