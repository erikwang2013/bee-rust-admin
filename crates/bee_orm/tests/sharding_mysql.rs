// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "mysql")]
//! Key-required sharding against a real MySQL server: two table shards inside
//! one database, and the same logical table split across two databases. Each
//! test is a no-op (with a skip notice) unless `BEE_ORM_MYSQL_DSN` is set; CI
//! sets it against the MySQL 5.7 / 8.0 / 8.4 / 9 matrix and MariaDB 10.11.
mod common;

use std::sync::Arc;

use bee_orm::pool::mysql::Pool;
use bee_orm::shard::{HashRouter, ShardedPools};
use bee_orm::{Db, MigrateOptions, Model, OrmError, Value};

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "shard_orders")]
struct ShardOrder {
    #[bee(pk, auto)]
    id: i64,
    user_id: i64,
    name: String,
}

fn with_database(dsn: &str, db: &str) -> String {
    // shortcut: DSN path swap ignores query params; upgrade if the CI DSN gains them.
    let base = dsn.rsplit_once('/').map(|(base, _)| base).unwrap_or(dsn);
    format!("{base}/{db}")
}

async fn admin() -> Result<Option<(Pool, String)>, OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(None) };
    let admin = Pool::connect(&dsn, 1)?;
    Ok(Some((admin, dsn)))
}

async fn count(pool: &Pool, table: &str) -> Result<i64, OrmError> {
    let rows = pool.query(&format!("SELECT COUNT(*) AS n FROM {table}"), &[]).await?;
    Ok(rows[0]["n"].as_i64().unwrap())
}

async fn names(pool: &Pool, table: &str) -> Result<Vec<String>, OrmError> {
    let rows = pool.query(&format!("SELECT name FROM {table} ORDER BY id"), &[]).await?;
    Ok(rows.iter().filter_map(|row| row["name"].as_str().map(str::to_owned)).collect())
}

/// `HashRouter::new(1, 2)`: one database, two physical tables. Each keyed
/// handle must rewrite the logical name and see only its own table's row.
#[tokio::test]
async fn table_shards_round_trip() -> Result<(), OrmError> {
    let Some((admin, dsn)) = admin().await? else { return Ok(()) };
    admin.execute("DROP TABLE IF EXISTS shard_orders_0, shard_orders_1", &[]).await?;

    let shards =
        ShardedPools::new(vec![Pool::connect(&dsn, 2)?], Arc::new(HashRouter::new(1, 2)?))?;
    shards.sync_all::<ShardOrder>(MigrateOptions::default()).await?;

    let first = shards.for_key(Value::Int(1))?;
    let second = shards.for_key(Value::Int(2))?;
    assert_eq!(first.resolve_table("shard_orders")?, "shard_orders_0");
    assert_eq!(second.resolve_table("shard_orders")?, "shard_orders_1");

    ShardOrder { id: 0, user_id: 1, name: "alice".into() }.insert(&first).await?;
    let rows = ShardOrder::query().all(&first).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "alice");
    let other = ShardOrder::query().all(&second).await?;
    assert!(other.is_empty(), "key 2 must not see key 1's row");

    ShardOrder { id: 0, user_id: 2, name: "bob".into() }.insert(&second).await?;
    let rows = ShardOrder::query().all(&second).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "bob");

    assert_eq!(count(&admin, "shard_orders_0").await?, 1);
    assert_eq!(count(&admin, "shard_orders_1").await?, 1);

    admin.execute("DROP TABLE IF EXISTS shard_orders_0, shard_orders_1", &[]).await?;
    Ok(())
}

/// `HashRouter::new(2, 1)`: the logical table lives once in each of two
/// databases. Keys 1 and 2 route to different pools while keeping the plain
/// table name, and each database must hold exactly its own row.
#[tokio::test]
async fn database_shards_round_trip() -> Result<(), OrmError> {
    let Some((admin, dsn)) = admin().await? else { return Ok(()) };
    admin.execute("CREATE DATABASE IF NOT EXISTS bee_shard_0", &[]).await?;
    admin.execute("CREATE DATABASE IF NOT EXISTS bee_shard_1", &[]).await?;

    let p0 = Pool::connect(&with_database(&dsn, "bee_shard_0"), 2)?;
    let p1 = Pool::connect(&with_database(&dsn, "bee_shard_1"), 2)?;
    admin.execute("DROP TABLE IF EXISTS bee_shard_0.shard_orders", &[]).await?;
    admin.execute("DROP TABLE IF EXISTS bee_shard_1.shard_orders", &[]).await?;

    let shards = ShardedPools::new(vec![p0.clone(), p1.clone()], Arc::new(HashRouter::new(2, 1)?))?;
    shards.sync_all::<ShardOrder>(MigrateOptions::default()).await?;

    let first = shards.for_key(Value::Int(1))?;
    let second = shards.for_key(Value::Int(2))?;
    assert_eq!(first.resolve_table("shard_orders")?, "shard_orders");
    assert_eq!(second.resolve_table("shard_orders")?, "shard_orders");

    ShardOrder { id: 0, user_id: 1, name: "alice".into() }.insert(&first).await?;
    ShardOrder { id: 0, user_id: 2, name: "bob".into() }.insert(&second).await?;

    // Each handle reads its own database only; the per-pool names pin the
    // routing direction, which 1/1 counts alone would not.
    let rows = ShardOrder::query().all(&first).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "alice");
    let rows = ShardOrder::query().all(&second).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "bob");
    assert_eq!(names(&p0, "shard_orders").await?, vec!["alice"], "key 1 lands in bee_shard_0");
    assert_eq!(names(&p1, "shard_orders").await?, vec!["bob"], "key 2 lands in bee_shard_1");

    assert_eq!(count(&admin, "bee_shard_0.shard_orders").await?, 1);
    assert_eq!(count(&admin, "bee_shard_1.shard_orders").await?, 1);

    admin.execute("DROP TABLE IF EXISTS bee_shard_0.shard_orders", &[]).await?;
    admin.execute("DROP TABLE IF EXISTS bee_shard_1.shard_orders", &[]).await?;
    Ok(())
}
