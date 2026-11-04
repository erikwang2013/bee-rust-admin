// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Key-required sharding: a [`ShardRouter`] maps a shard key to a database
//! ordinal and a physical table name, and [`ShardedPools`] presents the
//! selected pool behind the regular [`Db`] seam.
//!
//! Every ORM statement that addresses a model table goes through
//! [`Db::resolve_table`], so a [`ShardHandle`] is a drop-in `Db` that rewrites
//! the logical table name for its key. Physical names are spliced into SQL
//! unquoted, so they are validated here.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{Db, Dialect, MigrateOptions, Model, OrmError, Result, Row, Value, migrate};

/// Maps shard keys to a database ordinal and a physical table name.
///
/// All routing is key-required: a statement without a key cannot pick a
/// shard, so there is no default database or table. Both shard counts must be
/// at least 1; [`ShardedPools::new`] rejects a router that reports 0.
pub trait ShardRouter: Send + Sync {
    /// The number of database shards the pool set covers.
    fn db_shards(&self) -> usize;

    /// The number of table shards per logical table.
    fn table_shards(&self) -> usize;

    /// The database ordinal for `key`. May be out of range on a
    /// misconfigured router; [`ShardedPools::for_key`] rejects that.
    fn db_index(&self, key: &Value) -> Result<usize>;

    /// The physical table for `logical` under `key`.
    fn table_name(&self, logical: &str, key: &Value) -> Result<String>;

    /// The physical table for `logical` at table ordinal `index` — what
    /// [`ShardedPools::sync_all`] enumerates. Must agree with
    /// [`Self::table_name`] for every index it can return.
    fn table_name_at(&self, logical: &str, index: usize) -> Result<String>;
}

/// FNV-1a 64 routing over one composite index:
/// `shard = hash % (db_shards * table_shards)`, `db = shard / table_shards`,
/// table ordinal = `shard % table_shards` (the logical name when
/// `table_shards == 1`). One index keeps every database/table pair reachable;
/// hashing each dimension independently would reach only
/// `lcm(db_shards, table_shards)` pairs.
pub struct HashRouter {
    db_shards: usize,
    table_shards: usize,
}

impl HashRouter {
    /// Both shard counts must be at least 1.
    pub fn new(db_shards: usize, table_shards: usize) -> Result<Self> {
        if db_shards == 0 || table_shards == 0 {
            return Err(OrmError::ShardError("sharding: shard counts must be at least 1".into()));
        }
        if db_shards.checked_mul(table_shards).is_none() {
            return Err(OrmError::ShardError("sharding: shard counts overflow".into()));
        }
        Ok(Self { db_shards, table_shards })
    }

    /// The composite shard ordinal: one hash spans every database/table pair.
    fn shard_index(&self, key: &Value) -> Result<u64> {
        Ok(key_hash(key)? % (self.db_shards as u64 * self.table_shards as u64))
    }
}

impl ShardRouter for HashRouter {
    fn db_shards(&self) -> usize {
        self.db_shards
    }

    fn table_shards(&self) -> usize {
        self.table_shards
    }

    fn db_index(&self, key: &Value) -> Result<usize> {
        Ok((self.shard_index(key)? / self.table_shards as u64) as usize)
    }

    fn table_name(&self, logical: &str, key: &Value) -> Result<String> {
        let index = (self.shard_index(key)? % self.table_shards as u64) as usize;
        self.table_name_at(logical, index)
    }

    fn table_name_at(&self, logical: &str, index: usize) -> Result<String> {
        if index >= self.table_shards {
            return Err(OrmError::ShardError(format!(
                "sharding: table index {index} out of range for `{logical}` \
                 with {} table shards",
                self.table_shards
            )));
        }
        Ok(if self.table_shards == 1 { logical.to_owned() } else { format!("{logical}_{index}") })
    }
}

/// FNV-1a 64 over the key's canonical bytes: `Int` as i64 little-endian,
/// `Text` as UTF-8, `Bytes` raw, `Bool` as one byte. Other key types are a
/// loud error — never a silent hash.
pub(crate) fn key_hash(key: &Value) -> Result<u64> {
    let bytes: Vec<u8> = match key {
        Value::Int(value) => value.to_le_bytes().to_vec(),
        Value::Text(value) => value.as_bytes().to_vec(),
        Value::Bytes(value) => value.clone(),
        Value::Bool(value) => vec![u8::from(*value)],
        _ => {
            return Err(OrmError::ShardError(
                "sharding: shard key must be Int, Text, Bytes, or Bool".into(),
            ));
        }
    };
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(hash)
}

/// Reject a physical name that would be spliced into SQL unquoted.
fn validate_table_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(OrmError::ShardError(format!("sharding: invalid physical table name `{name}`")))
    }
}

/// The pools plus the router that picks among them.
pub struct ShardedPools<P> {
    pools: Vec<P>,
    router: Arc<dyn ShardRouter>,
}

impl<P> ShardedPools<P> {
    /// `pools` must be non-empty and match [`ShardRouter::db_shards`].
    pub fn new(pools: Vec<P>, router: Arc<dyn ShardRouter>) -> Result<Self> {
        if pools.is_empty() {
            return Err(OrmError::ShardError("sharding: at least one pool is required".into()));
        }
        if router.db_shards() != pools.len() {
            return Err(OrmError::ShardError(format!(
                "sharding: router has {} database shards but {} pools were given",
                router.db_shards(),
                pools.len()
            )));
        }
        if router.db_shards() == 0 || router.table_shards() == 0 {
            return Err(OrmError::ShardError(
                "sharding: router must declare at least 1 database and table shard".into(),
            ));
        }
        Ok(Self { pools, router })
    }

    /// Every configured pool, in router ordinal order — the escape hatch for
    /// operations and transactions that span shards. Keyed statements must
    /// still go through [`Self::for_key`]; a pool alone cannot pick a table.
    pub fn pools(&self) -> &[P] {
        &self.pools
    }

    /// The pool at database ordinal `index`, or `None` if out of range.
    /// Same escape-hatch caveat as [`Self::pools`].
    pub fn pool(&self, index: usize) -> Option<&P> {
        self.pools.get(index)
    }

    /// Every configured pool, in router ordinal order, as an iterator — for
    /// operations that must run on every shard. Keyed statements still go
    /// through [`Self::for_key`] / [`Self::for_model`]: a pool alone cannot
    /// pick a table.
    pub fn all_shards(&self) -> impl Iterator<Item = &P> {
        self.pools.iter()
    }

    /// The number of database shards.
    pub fn len(&self) -> usize {
        self.pools.len()
    }

    /// Whether no pools are configured. Always `false` for a value built by
    /// [`Self::new`], which requires at least one pool.
    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
    }

    /// The handle for `key`'s database shard.
    pub fn for_key(&self, key: impl Into<Value>) -> Result<ShardHandle<'_, P>> {
        let key = key.into();
        let index = self.router.db_index(&key)?;
        let Some(pool) = self.pools.get(index) else {
            return Err(OrmError::ShardError(format!(
                "sharding: router selected database shard {index}, out of range for {} pools",
                self.pools.len()
            )));
        };
        Ok(ShardHandle { pool, router: &*self.router, key })
    }

    /// The handle for `model`'s database shard, routed on
    /// [`Model::shard_key`]. A model without a `#[bee(shard_key)]` field, or
    /// one whose key is `None` / `NULL`, has no usable key, so this returns
    /// [`OrmError::ShardError`] rather than guessing a shard.
    pub fn for_model<M: Model>(&self, model: &M) -> Result<ShardHandle<'_, P>> {
        let key = model.shard_key().ok_or_else(|| {
            OrmError::ShardError(format!(
                "sharding: model `{}` has no shard key (mark a field #[bee(shard_key)]; \
                 a NULL key is not routable)",
                M::table_name()
            ))
        })?;
        self.for_key(key)
    }
}

impl<P: Db> ShardedPools<P> {
    /// [`migrate::sync_with`] on every physical table of every pool: each
    /// pool × each table ordinal `0..table_shards`. Returns the total number
    /// of columns added across all targets. This serial path is the default
    /// (see `sync_all_parallel` for the caveat).
    pub async fn sync_all<M: Model>(&self, options: MigrateOptions) -> Result<u64> {
        let mut added = 0;
        for pool in &self.pools {
            for table_index in 0..self.router.table_shards() {
                let target = ShardTarget { pool, router: &*self.router, table_index };
                added += migrate::sync_with::<M, _>(&target, options).await?;
            }
        }
        Ok(added)
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
impl<P: Db + Clone + Send + Sync + 'static> ShardedPools<P> {
    /// [`sync_all`](Self::sync_all) with one task per pool × table ordinal,
    /// run concurrently; returns the total number of columns added.
    ///
    /// [`sync_all`](Self::sync_all) remains the default: on MySQL every
    /// `ALTER TABLE` takes a metadata lock, so concurrent DDL against one
    /// server serializes anyway and only adds lock-wait timeouts to the
    /// failure modes. Parallelism pays off when the pools are distinct
    /// servers. One task is spawned per target with no concurrency cap. The
    /// first error is returned; dropping the task set aborts the remaining
    /// targets. DDL already applied is not rolled back, so a failure leaves
    /// the shards partially migrated — rerun once the cause is fixed.
    pub async fn sync_all_parallel<M: Model>(&self, options: MigrateOptions) -> Result<u64> {
        let mut tasks = tokio::task::JoinSet::new();
        for pool in &self.pools {
            for table_index in 0..self.router.table_shards() {
                let target = OwnedShardTarget {
                    pool: pool.clone(),
                    router: Arc::clone(&self.router),
                    table_index,
                };
                tasks.spawn(async move { migrate::sync_with::<M, _>(&target, options).await });
            }
        }
        let mut added = 0;
        while let Some(result) = tasks.join_next().await {
            added += result.map_err(|error| {
                OrmError::ShardError(format!("sharding: sync task failed: {error}"))
            })??;
        }
        Ok(added)
    }
}

/// A [`Db`] bound to one pool and one shard key.
pub struct ShardHandle<'a, P> {
    pool: &'a P,
    router: &'a dyn ShardRouter,
    key: Value,
}

impl<'a, P> ShardHandle<'a, P> {
    /// The pool this handle executes on.
    pub fn pool(&self) -> &'a P {
        self.pool
    }
}

#[async_trait]
impl<P: Db> Db for ShardHandle<'_, P> {
    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        self.pool.query(sql, params).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        self.pool.execute(sql, params).await
    }

    fn dialect(&self) -> Option<Dialect> {
        self.pool.dialect()
    }

    fn resolve_table(&self, logical: &str) -> Result<String> {
        let name = self.router.table_name(logical, &self.key)?;
        validate_table_name(&name)?;
        Ok(name)
    }

    /// Forwarded explicitly: the default would lose mysql's read-back.
    async fn insert_returning(
        &self,
        table: &str,
        pk_column: &str,
        sql: &str,
        params: &[Value],
    ) -> Result<Option<Row>> {
        self.pool.insert_returning(table, pk_column, sql, params).await
    }

    /// Forwarded explicitly: the default would drop the auto-increment hint
    /// and send MySQL back to probing `information_schema` on every create.
    async fn insert_returning_auto(
        &self,
        table: &str,
        pk_column: &str,
        auto_increment: Option<bool>,
        sql: &str,
        params: &[Value],
    ) -> Result<Option<Row>> {
        self.pool.insert_returning_auto(table, pk_column, auto_increment, sql, params).await
    }
}

/// One pool at one table ordinal, for [`ShardedPools::sync_all`].
struct ShardTarget<'a, P> {
    pool: &'a P,
    router: &'a dyn ShardRouter,
    table_index: usize,
}

#[async_trait]
impl<P: Db> Db for ShardTarget<'_, P> {
    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        self.pool.query(sql, params).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        self.pool.execute(sql, params).await
    }

    fn dialect(&self) -> Option<Dialect> {
        self.pool.dialect()
    }

    fn resolve_table(&self, logical: &str) -> Result<String> {
        let name = self.router.table_name_at(logical, self.table_index)?;
        validate_table_name(&name)?;
        Ok(name)
    }
}

/// One pool at one table ordinal, owned for [`ShardedPools::sync_all_parallel`]
/// (`'static` task bodies cannot borrow the pools).
#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
struct OwnedShardTarget<P> {
    pool: P,
    router: Arc<dyn ShardRouter>,
    table_index: usize,
}

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
#[async_trait]
impl<P: Db> Db for OwnedShardTarget<P> {
    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        self.pool.query(sql, params).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        self.pool.execute(sql, params).await
    }

    fn dialect(&self) -> Option<Dialect> {
        self.pool.dialect()
    }

    fn resolve_table(&self, logical: &str) -> Result<String> {
        let name = self.router.table_name_at(logical, self.table_index)?;
        validate_table_name(&name)?;
        Ok(name)
    }
}
