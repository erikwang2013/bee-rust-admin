// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Routing vectors, handle forwarding and shard-wide migration checks.

use super::*;
use crate::MigrateOptions;
use crate::shard::{HashRouter, ShardRouter, ShardedPools, key_hash};
use async_trait::async_trait;
use std::sync::Arc;

#[test]
fn fnv1a_matches_known_vectors() {
    assert_eq!(key_hash(&Value::Text("a".into())).unwrap(), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(key_hash(&Value::Text("foobar".into())).unwrap(), 0x8594_4171_f739_67e8);
}

#[test]
fn hash_router_routes_known_keys() {
    let router = HashRouter::new(2, 4).unwrap();
    assert_eq!(router.db_index(&Value::Int(1)).unwrap(), 1);
    assert_eq!(router.table_name("users", &Value::Int(1)).unwrap(), "users_0");
    assert_eq!(router.db_index(&Value::Int(2)).unwrap(), 1);
    assert_eq!(router.table_name("users", &Value::Int(2)).unwrap(), "users_3");
}

#[test]
fn mixed_shard_counts_reach_every_database_table_pair() {
    let router = HashRouter::new(2, 4).unwrap();
    let seen: std::collections::HashSet<(usize, String)> = (0..256)
        .map(|key| {
            let key = Value::Int(key);
            (router.db_index(&key).unwrap(), router.table_name("users", &key).unwrap())
        })
        .collect();
    assert_eq!(seen.len(), 8);
}

#[test]
fn one_table_shard_keeps_the_logical_name() {
    let router = HashRouter::new(2, 1).unwrap();
    assert_eq!(router.table_name("users", &Value::Int(2)).unwrap(), "users");
    assert_eq!(router.table_name_at("users", 0).unwrap(), "users");
}

#[test]
fn table_name_at_rejects_out_of_range_indices() {
    let router = HashRouter::new(1, 3).unwrap();
    assert_eq!(router.table_name_at("users", 2).unwrap(), "users_2");
    assert!(router.table_name_at("users", 3).is_err());
    let router = HashRouter::new(2, 1).unwrap();
    assert!(router.table_name_at("users", 1).is_err());
}

#[test]
fn unsupported_keys_are_rejected() {
    for key in [Value::Null, Value::Float(1.0), Value::Json(serde_json::json!({}))] {
        assert!(key_hash(&key).is_err(), "{key:?}");
    }
}

#[test]
fn zero_shards_are_rejected() {
    assert!(HashRouter::new(0, 1).is_err());
    assert!(HashRouter::new(1, 0).is_err());
}

#[test]
fn hash_router_rejects_overflowing_shard_counts() {
    assert!(HashRouter::new(usize::MAX, 2).is_err());
}

#[tokio::test]
async fn for_key_selects_the_router_database() {
    let a = Mock::default();
    let b = Mock::default();
    let (a_probe, b_probe) = (a.clone(), b.clone());
    let shards = ShardedPools::new(vec![a, b], Arc::new(HashRouter::new(2, 1).unwrap())).unwrap();

    shards.for_key(Value::Int(1)).unwrap().execute("SELECT 1", &[]).await.unwrap();
    assert_eq!(a_probe.calls().len(), 1);
    assert_eq!(b_probe.calls().len(), 0);

    shards.for_key(Value::Int(2)).unwrap().execute("SELECT 1", &[]).await.unwrap();
    assert_eq!(b_probe.calls().len(), 1);
}

#[tokio::test]
async fn sync_all_creates_every_physical_table_in_every_pool() {
    let a = Mock::with_dialect(Dialect::Sqlite);
    let b = Mock::with_dialect(Dialect::Sqlite);
    let (a_probe, b_probe) = (a.clone(), b.clone());
    let shards = ShardedPools::new(vec![a, b], Arc::new(HashRouter::new(2, 2).unwrap())).unwrap();

    // Two columns per physical table, two tables per pool, two pools.
    assert_eq!(shards.sync_all::<Author>(MigrateOptions::default()).await.unwrap(), 8);

    let created = |probe: &Mock| -> Vec<String> {
        probe
            .calls()
            .into_iter()
            .map(|(sql, _)| sql)
            .filter(|sql| sql.starts_with("CREATE TABLE"))
            .collect()
    };
    let authors_0 = "CREATE TABLE IF NOT EXISTS authors_0 (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                     deleted INTEGER NOT NULL DEFAULT 0)"
        .to_string();
    let authors_1 = authors_0.replace("authors_0", "authors_1");
    assert_eq!(created(&a_probe), vec![authors_0.clone(), authors_1.clone()]);
    assert_eq!(created(&b_probe), vec![authors_0, authors_1]);
}

#[test]
fn pool_accessors_expose_the_configured_pools() {
    let shards = ShardedPools::new(
        vec![Mock::with_dialect(Dialect::Mysql), Mock::with_dialect(Dialect::Sqlite)],
        Arc::new(HashRouter::new(2, 1).unwrap()),
    )
    .unwrap();

    let pools = shards.pools();
    assert_eq!(pools.len(), 2);
    assert_eq!(pools[0].dialect(), Some(Dialect::Mysql));
    assert_eq!(pools[1].dialect(), Some(Dialect::Sqlite));

    assert_eq!(shards.pool(0).unwrap().dialect(), Some(Dialect::Mysql));
    assert_eq!(shards.pool(1).unwrap().dialect(), Some(Dialect::Sqlite));
    assert!(shards.pool(2).is_none());

    assert_eq!(shards.len(), 2);
    assert!(!shards.is_empty());
}

#[test]
fn sharded_pools_rejects_pool_count_mismatch() {
    let result = ShardedPools::new(vec![Mock::default()], Arc::new(HashRouter::new(2, 1).unwrap()));
    assert!(result.is_err());
}

struct ZeroTableRouter;
impl ShardRouter for ZeroTableRouter {
    fn db_shards(&self) -> usize {
        1
    }
    fn table_shards(&self) -> usize {
        0
    }
    fn db_index(&self, _: &Value) -> Result<usize> {
        Ok(0)
    }
    fn table_name(&self, logical: &str, _: &Value) -> Result<String> {
        Ok(logical.to_owned())
    }
    fn table_name_at(&self, logical: &str, _: usize) -> Result<String> {
        Ok(logical.to_owned())
    }
}

#[test]
fn sharded_pools_rejects_a_router_with_zero_table_shards() {
    let result = ShardedPools::new(vec![Mock::default()], Arc::new(ZeroTableRouter));
    assert!(result.is_err());
}

struct BadRouter;
impl ShardRouter for BadRouter {
    fn db_shards(&self) -> usize {
        1
    }
    fn table_shards(&self) -> usize {
        1
    }
    fn db_index(&self, _: &Value) -> Result<usize> {
        Ok(0)
    }
    fn table_name(&self, _: &str, _: &Value) -> Result<String> {
        Ok("users; DROP TABLE x".into())
    }
    fn table_name_at(&self, _: &str, _: usize) -> Result<String> {
        Ok("users; DROP TABLE x".into())
    }
}

#[tokio::test]
async fn invalid_physical_table_name_is_rejected() {
    let shards = ShardedPools::new(vec![Mock::default()], Arc::new(BadRouter)).unwrap();
    let handle = shards.for_key(Value::Int(1)).unwrap();
    assert!(handle.resolve_table("users").is_err());
}

struct OutOfRangeRouter;
impl ShardRouter for OutOfRangeRouter {
    fn db_shards(&self) -> usize {
        1
    }
    fn table_shards(&self) -> usize {
        1
    }
    fn db_index(&self, _: &Value) -> Result<usize> {
        Ok(1)
    }
    fn table_name(&self, logical: &str, _: &Value) -> Result<String> {
        Ok(logical.to_owned())
    }
    fn table_name_at(&self, logical: &str, _: usize) -> Result<String> {
        Ok(logical.to_owned())
    }
}

#[test]
fn for_key_rejects_out_of_range_router_indices() {
    let shards = ShardedPools::new(vec![Mock::default()], Arc::new(OutOfRangeRouter)).unwrap();
    assert!(shards.for_key(Value::Int(1)).is_err());
}

struct ReturningDb;
#[async_trait]
impl Db for ReturningDb {
    async fn query(&self, _: &str, _: &[Value]) -> Result<Vec<Row>> {
        Ok(Vec::new())
    }
    async fn execute(&self, _: &str, _: &[Value]) -> Result<u64> {
        Ok(1)
    }
    async fn insert_returning(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &[Value],
    ) -> Result<Option<Row>> {
        Ok(Some(row_of(&[("id", serde_json::json!(1))])))
    }
}

#[tokio::test]
async fn insert_returning_delegates_to_the_selected_pool() {
    let shards =
        ShardedPools::new(vec![ReturningDb], Arc::new(HashRouter::new(1, 1).unwrap())).unwrap();
    let handle = shards.for_key(Value::Int(1)).unwrap();
    let row = Db::insert_returning(&handle, "users", "id", "INSERT ...", &[]).await.unwrap();
    assert!(row.is_some());
}

#[derive(Clone, Default)]
struct AutoHintDb {
    hint: std::sync::Arc<std::sync::Mutex<Option<Option<bool>>>>,
}

#[async_trait]
impl Db for AutoHintDb {
    async fn query(&self, _: &str, _: &[Value]) -> Result<Vec<Row>> {
        Ok(Vec::new())
    }
    async fn execute(&self, _: &str, _: &[Value]) -> Result<u64> {
        Ok(1)
    }
    async fn insert_returning_auto(
        &self,
        _: &str,
        _: &str,
        auto_increment: Option<bool>,
        _: &str,
        _: &[Value],
    ) -> Result<Option<Row>> {
        *self.hint.lock().unwrap() = Some(auto_increment);
        Ok(Some(row_of(&[("id", serde_json::json!(1))])))
    }
}

#[tokio::test]
async fn insert_returning_auto_forwards_the_hint_to_the_selected_pool() {
    let db = AutoHintDb::default();
    let probe = db.clone();
    let shards = ShardedPools::new(vec![db], Arc::new(HashRouter::new(1, 1).unwrap())).unwrap();
    let handle = shards.for_key(Value::Int(1)).unwrap();

    let row = Db::insert_returning_auto(&handle, "users", "id", Some(true), "INSERT ...", &[])
        .await
        .unwrap();
    assert!(row.is_some());
    assert_eq!(*probe.hint.lock().unwrap(), Some(Some(true)));
}

#[tokio::test]
async fn dialect_is_forwarded() {
    let shards = ShardedPools::new(
        vec![Mock::with_dialect(Dialect::Mysql)],
        Arc::new(HashRouter::new(1, 1).unwrap()),
    )
    .unwrap();
    assert_eq!(shards.for_key(Value::Int(1)).unwrap().dialect(), Some(Dialect::Mysql));
}

/// A derive-generated model with a shard key. `crate = "crate"` because the
/// default expansion names `bee_orm`, which is not in this crate's own extern
/// prelude; ORM users outside the crate keep the default path.
#[derive(bee_orm_macro::Model)]
#[bee(crate = "crate", table = "tenant_rows")]
struct TenantRow {
    #[bee(pk)]
    id: i64,
    #[bee(shard_key)]
    tenant: String,
}

#[derive(bee_orm_macro::Model)]
#[bee(crate = "crate", table = "nullable_tenant_rows")]
struct NullableTenantRow {
    #[bee(pk)]
    id: i64,
    #[bee(shard_key)]
    tenant: Option<String>,
}

#[test]
fn derived_models_expose_their_shard_key() {
    let row = TenantRow { id: 1, tenant: "acme".into() };
    assert_eq!(row.shard_key(), Some(Value::Text("acme".into())));
    // `Option<T>::None` collapses to Value::Null at codegen; a NULL key is
    // not routable, so `shard_key()` reports no key rather than Null.
    let null = NullableTenantRow { id: 1, tenant: None };
    assert_eq!(null.shard_key(), None);
    // Models without the attribute keep the trait default.
    assert_eq!(Tiny { id: 1, v: 2 }.shard_key(), None);
}

#[tokio::test]
async fn for_model_routes_on_the_derived_shard_key() {
    let router = HashRouter::new(2, 1).unwrap();
    let expected = router.db_index(&Value::Text("acme".into())).unwrap();
    let a = Mock::default();
    let b = Mock::default();
    let (a_probe, b_probe) = (a.clone(), b.clone());
    let shards = ShardedPools::new(vec![a, b], Arc::new(router)).unwrap();

    let model = TenantRow { id: 0, tenant: "acme".into() };
    shards.for_model(&model).unwrap().execute("SELECT 1", &[]).await.unwrap();

    assert_eq!(a_probe.calls().len(), usize::from(expected == 0));
    assert_eq!(b_probe.calls().len(), usize::from(expected == 1));
}

#[test]
fn for_model_without_a_shard_key_is_a_shard_error() {
    let shards =
        ShardedPools::new(vec![Mock::default()], Arc::new(HashRouter::new(1, 1).unwrap())).unwrap();
    assert!(matches!(
        shards.for_model(&Tiny { id: 1, v: 2 }),
        Err(OrmError::ShardError(ref message)) if message.contains("no shard key")
    ));
    assert!(matches!(
        shards.for_model(&NullableTenantRow { id: 1, tenant: None }),
        Err(OrmError::ShardError(ref message)) if message.contains("no shard key")
    ));
}

#[test]
fn all_shards_iterates_every_pool() {
    let shards = ShardedPools::new(
        vec![Mock::with_dialect(Dialect::Mysql), Mock::with_dialect(Dialect::Sqlite)],
        Arc::new(HashRouter::new(2, 1).unwrap()),
    )
    .unwrap();

    let dialects: Vec<Option<Dialect>> = shards.all_shards().map(|pool| pool.dialect()).collect();
    assert_eq!(dialects, vec![Some(Dialect::Mysql), Some(Dialect::Sqlite)]);
}

#[test]
fn shard_origin_errors_use_the_shard_variant() {
    assert!(matches!(HashRouter::new(0, 1), Err(OrmError::ShardError(_))));
    assert!(matches!(key_hash(&Value::Null), Err(OrmError::ShardError(_))));

    let shards = ShardedPools::new(vec![Mock::default()], Arc::new(BadRouter)).unwrap();
    assert!(matches!(
        shards.for_key(Value::Int(1)).unwrap().resolve_table("users"),
        Err(OrmError::ShardError(_))
    ));

    let shards = ShardedPools::new(vec![Mock::default()], Arc::new(OutOfRangeRouter)).unwrap();
    assert!(matches!(shards.for_key(Value::Int(1)), Err(OrmError::ShardError(_))));
}

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
#[tokio::test]
async fn sync_all_parallel_visits_every_pool_and_table() {
    let a = Mock::with_dialect(Dialect::Sqlite);
    let b = Mock::with_dialect(Dialect::Sqlite);
    let (a_probe, b_probe) = (a.clone(), b.clone());
    let shards = ShardedPools::new(vec![a, b], Arc::new(HashRouter::new(2, 2).unwrap())).unwrap();

    // Same total as the serial path: 2 pools × 2 tables × 2 missing columns.
    assert_eq!(shards.sync_all_parallel::<Author>(MigrateOptions::default()).await.unwrap(), 8);

    // Per-pool call order is nondeterministic; the targets visited are not.
    for probe in [&a_probe, &b_probe] {
        let calls = probe.calls();
        let created: Vec<&str> = calls
            .iter()
            .map(|(sql, _)| sql.as_str())
            .filter(|sql| sql.starts_with("CREATE TABLE"))
            .collect();
        assert_eq!(created.len(), 2, "{created:?}");
        assert!(created.iter().any(|sql| sql.contains("authors_0")), "{created:?}");
        assert!(created.iter().any(|sql| sql.contains("authors_1")), "{created:?}");
    }
}
