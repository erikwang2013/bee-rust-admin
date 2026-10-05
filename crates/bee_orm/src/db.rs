// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::error::{OrmError, normalize, validate_ident};
use crate::Model;
use sqlx::mysql::{MySql, MySqlPool, MySqlPoolOptions};
use sqlx::{Executor, QueryBuilder};

/// 连接池句柄；Clone 廉价（内部 Arc）。
#[derive(Clone)]
pub struct Db {
    pool: MySqlPool,
}

/// 事务；由 `Db::begin` 创建，`commit`/`rollback` 消费自身。
pub struct Tx {
    tx: sqlx::Transaction<'static, MySql>,
}

impl Db {
    pub async fn connect(dsn: &str) -> Result<Self, OrmError> {
        let pool = MySqlPoolOptions::new()
            .max_connections(10)
            .connect(dsn)
            .await
            .map_err(|e| OrmError::ConnectionError(e.to_string()))?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &MySqlPool {
        &self.pool
    }

    /// 执行写死的 SQL（DDL/运维/建测试库）。参数化查询一律走 QuerySet / CRUD。
    pub async fn exec_sql(&self, sql: &str) -> Result<u64, OrmError> {
        sqlx::query(sql)
            .execute(&self.pool)
            .await
            .map_err(normalize)
            .map(|r| r.rows_affected())
    }

    /// INSERT 并回填自增主键，返回自增 id（无自增列时返回 0）。
    pub async fn insert<T: Model>(&self, m: &mut T) -> Result<u64, OrmError> {
        insert_with(&self.pool, m).await
    }

    /// 按主键读一行。
    pub async fn read<T: Model>(&self, id: u64) -> Result<Option<T>, OrmError> {
        read_with(&self.pool, id).await
    }

    /// 按主键更新全列，返回受影响行数。
    pub async fn update<T: Model>(&self, m: &T) -> Result<u64, OrmError> {
        update_with(&self.pool, m).await
    }

    /// 按主键删除，返回受影响行数。
    pub async fn delete<T: Model>(&self, id: u64) -> Result<u64, OrmError> {
        delete_with::<_, T>(&self.pool, id).await
    }

    pub async fn begin(&self) -> Result<Tx, OrmError> {
        Ok(Tx {
            tx: self.pool.begin().await.map_err(normalize)?,
        })
    }

    /// 重设关联：事务内「删旧 + 逐条插入新值」。空集合 = 清空。
    /// `owner` 是 (关联列, 值)，`target` 是对端列名（如 admin_id/role_id、role_id/menu_id）。
    pub async fn set_relations(
        &self,
        table: &str,
        owner: (&str, u64),
        target: &str,
        ids: &[u64],
    ) -> Result<(), OrmError> {
        validate_ident(table)?;
        validate_ident(owner.0)?;
        validate_ident(target)?;
        let mut tx = self.pool.begin().await.map_err(normalize)?;
        sqlx::query(&format!("DELETE FROM {} WHERE {} = ?", table, owner.0))
            .bind(owner.1)
            .execute(&mut *tx)
            .await
            .map_err(normalize)?;
        if !ids.is_empty() {
            // ponytail: 逐条插入（管理后台量级：一次几十条），量大改多值 VALUES
            let sql = format!("INSERT INTO {} ({}, {}) VALUES (?, ?)", table, owner.0, target);
            for id in ids {
                sqlx::query(&sql)
                    .bind(owner.1)
                    .bind(*id)
                    .execute(&mut *tx)
                    .await
                    .map_err(normalize)?;
            }
        }
        tx.commit().await.map_err(normalize)?;
        Ok(())
    }

    /// 取关联目标 id 列表。
    pub async fn get_relations(
        &self,
        table: &str,
        owner: (&str, u64),
        target: &str,
    ) -> Result<Vec<u64>, OrmError> {
        validate_ident(table)?;
        validate_ident(owner.0)?;
        validate_ident(target)?;
        let sql = format!("SELECT {} FROM {} WHERE {} = ?", target, table, owner.0);
        let rows: Vec<(u64,)> = sqlx::query_as(&sql)
            .bind(owner.1)
            .fetch_all(&self.pool)
            .await
            .map_err(normalize)?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// 按列删关联，返回删除行数。
    pub async fn del_relations(&self, table: &str, col: &str, id: u64) -> Result<u64, OrmError> {
        validate_ident(table)?;
        validate_ident(col)?;
        let sql = format!("DELETE FROM {} WHERE {} = ?", table, col);
        let res = sqlx::query(&sql).bind(id).execute(&self.pool).await.map_err(normalize)?;
        Ok(res.rows_affected())
    }
}

impl Tx {
    pub async fn commit(self) -> Result<(), OrmError> {
        self.tx.commit().await.map_err(normalize)
    }

    pub async fn rollback(self) -> Result<(), OrmError> {
        self.tx.rollback().await.map_err(normalize)
    }

    pub async fn exec_sql(&mut self, sql: &str) -> Result<u64, OrmError> {
        sqlx::query(sql)
            .execute(&mut *self.tx)
            .await
            .map_err(normalize)
            .map(|r| r.rows_affected())
    }

    pub async fn insert<T: Model>(&mut self, m: &mut T) -> Result<u64, OrmError> {
        insert_with(&mut *self.tx, m).await
    }

    pub async fn read<T: Model>(&mut self, id: u64) -> Result<Option<T>, OrmError> {
        read_with(&mut *self.tx, id).await
    }

    pub async fn update<T: Model>(&mut self, m: &T) -> Result<u64, OrmError> {
        update_with(&mut *self.tx, m).await
    }

    pub async fn delete<T: Model>(&mut self, id: u64) -> Result<u64, OrmError> {
        delete_with::<_, T>(&mut *self.tx, id).await
    }
}

/// 非自增列名列表。
fn insert_columns<T: Model>() -> Vec<&'static str> {
    T::META.columns.iter().filter(|c| !c.auto).map(|c| c.name).collect()
}

/// 通用执行体：pool 与事务共用（`&MySqlPool` 与 `&mut MySqlConnection` 都满足 `Executor`）。
pub(crate) async fn insert_with<'e, E, T>(ex: E, m: &mut T) -> Result<u64, OrmError>
where
    E: Executor<'e, Database = MySql>,
    T: Model,
{
    let meta = T::META;
    validate_ident(meta.table)?;
    let cols = insert_columns::<T>();
    if cols.is_empty() {
        return Err(OrmError::Unsupported(format!("模型 {} 没有可插入的列", meta.table)));
    }
    let mut qb = QueryBuilder::<MySql>::new(format!(
        "INSERT INTO {} ({}) VALUES (",
        meta.table,
        cols.join(", ")
    ));
    qb = m.bind_insert(qb);
    qb.push(")");
    let res = qb.build().execute(ex).await.map_err(normalize)?;
    let id = res.last_insert_id();
    if meta.columns.iter().any(|c| c.auto) {
        m.set_auto_pk(id);
    }
    Ok(id)
}

pub(crate) async fn read_with<'e, E, T>(ex: E, id: u64) -> Result<Option<T>, OrmError>
where
    E: Executor<'e, Database = MySql>,
    T: Model,
{
    let meta = T::META;
    let pk = meta
        .pk
        .ok_or_else(|| OrmError::Unsupported(format!("模型 {} 没有主键，不能用 read", meta.table)))?;
    validate_ident(meta.table)?;
    validate_ident(pk)?;
    let sql = format!("SELECT * FROM {} WHERE {} = ?", meta.table, pk);
    let row = sqlx::query(&sql).bind(id).fetch_optional(ex).await.map_err(normalize)?;
    match row {
        Some(r) => T::from_row(&r).map(Some).map_err(normalize),
        None => Ok(None),
    }
}

pub(crate) async fn update_with<'e, E, T>(ex: E, m: &T) -> Result<u64, OrmError>
where
    E: Executor<'e, Database = MySql>,
    T: Model,
{
    let meta = T::META;
    let pk = meta
        .pk
        .ok_or_else(|| OrmError::Unsupported(format!("模型 {} 没有主键，不能用 update", meta.table)))?;
    validate_ident(meta.table)?;
    validate_ident(pk)?;
    let mut qb = QueryBuilder::<MySql>::new(format!("UPDATE {} ", meta.table));
    qb = m.bind_update(qb);
    let res = qb.build().execute(ex).await.map_err(normalize)?;
    Ok(res.rows_affected())
}

pub(crate) async fn delete_with<'e, E, T>(ex: E, id: u64) -> Result<u64, OrmError>
where
    E: Executor<'e, Database = MySql>,
    T: Model,
{
    let meta = T::META;
    let pk = meta
        .pk
        .ok_or_else(|| OrmError::Unsupported(format!("模型 {} 没有主键，不能用 delete", meta.table)))?;
    validate_ident(meta.table)?;
    validate_ident(pk)?;
    let sql = format!("DELETE FROM {} WHERE {} = ?", meta.table, pk);
    let res = sqlx::query(&sql).bind(id).execute(ex).await.map_err(normalize)?;
    Ok(res.rows_affected())
}
