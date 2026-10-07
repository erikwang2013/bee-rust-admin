// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! F-5 `Model::create` real-database pins (tester, read-only acceptance of the
//! new API): the returned instance must carry the *stored* primary key and
//! `auto_now_add` value, address the same row for an immediate get/update
//! closure, and keys must number 1 → 2 on a fresh table. One pin per backend;
//! pg/mysql are DSN-gated no-ops without their env vars.
#![cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
mod common;

use bee_orm::{Db, Model, OrmError, Value, migrate};

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_create_pin")]
struct Pin {
    #[bee(pk, auto)]
    id: i64,
    name: String,
    #[bee(auto_now_add)]
    created_at: i64,
}

/// Shared body for all three backends.
async fn create_pin_roundtrip<D: Db + ?Sized>(pool: &D) -> Result<(), OrmError> {
    pool.execute("DROP TABLE IF EXISTS it_create_pin", &[]).await?;
    migrate::sync::<Pin, _>(pool).await?;

    let first = Pin { id: 0, name: "a".into(), created_at: 0 }.create(pool).await?;
    assert_eq!(first.id, 1, "first database-assigned pk must be 1, got {}", first.id);
    assert!(
        first.created_at > 1_700_000_000,
        "auto_now_add must be backfilled, got {}",
        first.created_at
    );

    // Raw row: the returned instance must carry the stored pk and timestamp.
    let rows = pool
        .query("SELECT id, created_at FROM it_create_pin WHERE id = ?", &[Value::Int(first.id)])
        .await?;
    assert_eq!(rows.len(), 1, "the pk create() returned must address a stored row");
    assert_eq!(rows[0]["id"], first.id);
    assert_eq!(rows[0]["created_at"], first.created_at, "returned auto_now_add must equal storage");

    // Continuity: the second row gets the next key (1 → 2).
    let second = Pin { id: 0, name: "b".into(), created_at: 0 }.create(pool).await?;
    assert_eq!(second.id, 2, "second pk must be 2, got {}", second.id);
    assert!(second.created_at >= first.created_at);

    // Immediate get closure through the returned pk.
    let got = Pin::query().filter_eq("id", first.id)?.one(pool).await?.expect("row by returned pk");
    assert_eq!(got, first);

    // Immediate update closure through the returned pk.
    let mut edited = first.clone();
    edited.name = "a2".into();
    assert_eq!(edited.update(pool).await?, 1);
    let reread =
        Pin::query().filter_eq("id", first.id)?.one(pool).await?.expect("row after update");
    assert_eq!(reread.name, "a2");
    assert_eq!(reread.created_at, first.created_at, "update must not disturb auto_now_add");
    Ok(())
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn create_returns_the_stored_row_sqlite() -> Result<(), OrmError> {
    use bee_orm::pool::sqlite::Pool;
    let path = std::env::temp_dir().join(format!("it_create_pin_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let pool = Pool::connect(path.to_str().unwrap(), 1)?;
    let result = create_pin_roundtrip(&pool).await;
    drop(pool);
    let _ = std::fs::remove_file(&path);
    result
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn create_returns_the_stored_row_postgres() -> Result<(), OrmError> {
    use bee_orm::pool::postgres::Pool;
    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 2)?;
    create_pin_roundtrip(&pool).await
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn create_returns_the_stored_row_mysql() -> Result<(), OrmError> {
    use bee_orm::pool::mysql::Pool;
    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 2)?;
    create_pin_roundtrip(&pool).await
}
