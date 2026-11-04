// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "sqlite")]
//! Key-required sharding over two in-memory SQLite pools: sync, routing and
//! cross-shard isolation with no external service.
use std::sync::Arc;

use bee_orm::pool::sqlite::Pool;
use bee_orm::shard::{HashRouter, ShardedPools};
use bee_orm::{MigrateOptions, Model, OrmError, Value};

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "shard_users")]
struct ShardUser {
    #[bee(pk, auto)]
    id: i64,
    user_id: i64,
    name: String,
}

async fn setup() -> (ShardedPools<Pool>, Pool, Pool, Arc<HashRouter>) {
    let p0 = Pool::connect(":memory:", 1).unwrap();
    let p1 = Pool::connect(":memory:", 1).unwrap();
    let router = Arc::new(HashRouter::new(2, 4).unwrap());
    let shards = ShardedPools::new(vec![p0.clone(), p1.clone()], router.clone()).unwrap();
    shards.sync_all::<ShardUser>(MigrateOptions::default()).await.unwrap();
    (shards, p0, p1, router)
}

#[tokio::test]
async fn sync_all_creates_every_physical_table() {
    let (_shards, p0, p1, _router) = setup().await;
    for pool in [&p0, &p1] {
        let rows = pool
            .query(
                "SELECT name FROM sqlite_master WHERE type = 'table' \
                 AND name LIKE 'shard_users_%' ORDER BY name",
                &[],
            )
            .await
            .unwrap();
        let names: Vec<&str> = rows.iter().filter_map(|r| r["name"].as_str()).collect();
        assert_eq!(names, vec!["shard_users_0", "shard_users_1", "shard_users_2", "shard_users_3"]);
    }
}

#[tokio::test]
async fn keyed_crud_stays_on_the_routed_shard() {
    let (shards, p0, p1, _router) = setup().await;

    // Composite 2×4 routing, both positive routes exercised below:
    // Int(5) -> pool 0 / shard_users_0; Int(1) -> pool 1 / shard_users_0
    // (same physical table name in two distinct databases).
    let db1 = shards.for_key(Value::Int(5)).unwrap();
    let user = ShardUser { id: 0, user_id: 5, name: "alice".into() };
    user.insert(&db1).await.unwrap();

    let found = ShardUser::query().filter_eq("user_id", 5).unwrap().all(&db1).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "alice");

    let db2 = shards.for_key(Value::Int(1)).unwrap();
    let other = ShardUser::query().filter_eq("user_id", 5).unwrap().all(&db2).await.unwrap();
    assert!(other.is_empty(), "key 1 must not see key 5's row");

    assert_eq!(count(&p0, "shard_users_0").await, 1);

    // Positive half: bob is written and read back through the other route,
    // landing on the same physical table name in the other database.
    let bob = ShardUser { id: 0, user_id: 1, name: "bob".into() };
    bob.insert(&db2).await.unwrap();
    let found = ShardUser::query().filter_eq("user_id", 1).unwrap().all(&db2).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "bob");

    assert_eq!(count(&p1, "shard_users_0").await, 1);
    assert_eq!(count(&p0, "shard_users_0").await, 1);
}

#[tokio::test]
async fn unsupported_key_is_an_error() {
    let (shards, _p0, _p1, _router) = setup().await;
    assert!(matches!(shards.for_key(Value::Null), Err(OrmError::ShardError(_))));
}

#[tokio::test]
async fn sync_all_is_idempotent() {
    let (shards, _p0, _p1, _router) = setup().await;
    assert_eq!(shards.sync_all::<ShardUser>(MigrateOptions::default()).await.unwrap(), 0);
}

async fn count(pool: &Pool, table: &str) -> i64 {
    let rows = pool.query(&format!("SELECT COUNT(*) AS n FROM {table}"), &[]).await.unwrap();
    rows[0]["n"].as_i64().unwrap()
}
