// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::error::{OrmError, normalize};
use sqlx::mysql::{MySqlPool, MySqlPoolOptions};

/// 连接池句柄；Clone 廉价（内部 Arc）。
#[derive(Clone)]
pub struct Db {
    pool: MySqlPool,
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
}
