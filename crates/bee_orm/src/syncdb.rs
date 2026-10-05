// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! syncdb：模型元数据 ↔ information_schema 对比，只增不删（Safe 模式）。
use crate::db::Db;
use crate::error::{OrmError, normalize, validate_ident};
use crate::meta::{ColumnMeta, ColumnType, ModelMeta, SyncdbMode};
use std::collections::HashSet;

pub fn index_name(table: &str, col: &str) -> String {
    format!("idx_{table}_{col}")
}

pub fn unique_index_name(table: &str, col: &str) -> String {
    format!("uk_{table}_{col}")
}

fn column_type_sql(c: &ColumnMeta) -> String {
    match c.ty {
        ColumnType::U64 => "BIGINT UNSIGNED".into(),
        ColumnType::I64 => "BIGINT".into(),
        ColumnType::U32 => "INT UNSIGNED".into(),
        ColumnType::I32 => "INT".into(),
        ColumnType::I16 => "SMALLINT".into(),
        ColumnType::I8 => "TINYINT".into(),
        ColumnType::Bool => "TINYINT(1)".into(),
        ColumnType::String => {
            if c.text {
                "TEXT".into()
            } else {
                format!("VARCHAR({})", c.len.unwrap_or(255))
            }
        }
        ColumnType::F64 => "DOUBLE".into(),
        ColumnType::DateTime => "DATETIME".into(),
        ColumnType::Json => "JSON".into(),
    }
}

/// 列定义（名 + 类型 + 约束）。可空列不加任何约束。
fn column_def(c: &ColumnMeta) -> String {
    let mut s = format!("{} {}", c.name, column_type_sql(c));
    if c.auto {
        s.push_str(" AUTO_INCREMENT NOT NULL");
    } else if c.nullable {
        // 可空：不加约束
    } else {
        match c.ty {
            ColumnType::String if c.text => s.push_str(" NOT NULL"),
            ColumnType::String => s.push_str(" NOT NULL DEFAULT ''"),
            // TEXT/JSON/DATETIME 不给默认值（JSON 列 MySQL 8 不允许字面量默认）
            ColumnType::Json | ColumnType::DateTime => s.push_str(" NOT NULL"),
            _ => s.push_str(" NOT NULL DEFAULT 0"),
        }
    }
    s
}

pub fn create_table_sql(m: &ModelMeta) -> String {
    let mut parts: Vec<String> = m.columns.iter().map(column_def).collect();
    if let Some(pk) = m.pk {
        parts.push(format!("PRIMARY KEY ({pk})"));
    }
    for c in m.columns.iter().filter(|c| c.unique) {
        parts.push(format!("UNIQUE KEY {} ({})", unique_index_name(m.table, c.name), c.name));
    }
    for c in m.columns.iter().filter(|c| c.index && !c.unique) {
        parts.push(format!("KEY {} ({})", index_name(m.table, c.name), c.name));
    }
    format!(
        "CREATE TABLE IF NOT EXISTS {} (\n  {}\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        m.table,
        parts.join(",\n  ")
    )
}

pub fn add_column_sql(table: &str, c: &ColumnMeta) -> String {
    format!("ALTER TABLE {table} ADD COLUMN {}", column_def(c))
}

pub fn create_index_sql(table: &str, c: &ColumnMeta) -> String {
    if c.unique {
        format!("CREATE UNIQUE INDEX {} ON {} ({})", unique_index_name(table, c.name), table, c.name)
    } else {
        format!("CREATE INDEX {} ON {} ({})", index_name(table, c.name), table, c.name)
    }
}

impl Db {
    /// 把一批模型同步到当前库（Safe：建表 / 加列 / 补索引，绝不删改）。
    /// 返回实际执行的 DDL 列表（空 = 已是最新）。
    pub async fn syncdb(&self, metas: &[ModelMeta], mode: SyncdbMode) -> Result<Vec<String>, OrmError> {
        if mode == SyncdbMode::Force {
            return Err(OrmError::Unsupported("SyncdbMode::Force 未实现（本期 Safe）".into()));
        }
        let mut executed = Vec::new();
        for m in metas {
            validate_ident(m.table)?;
            let exists: Option<(String,)> = sqlx::query_as(
                // CAST：information_schema 的名称列是 utf8mb3_bin，协议层带 BINARY 标志，
                // sqlx 会判成 VARBINARY 而拒绝解码为 String。
                "SELECT CAST(table_name AS CHAR) FROM information_schema.tables \
                 WHERE table_schema = DATABASE() AND table_name = ?",
            )
            .bind(m.table)
            .fetch_optional(self.pool())
            .await
            .map_err(normalize)?;

            if exists.is_none() {
                let ddl = create_table_sql(m);
                self.exec_sql(&ddl).await?;
                executed.push(ddl);
                continue;
            }

            let cols: Vec<(String,)> = sqlx::query_as(
                "SELECT CAST(column_name AS CHAR) FROM information_schema.columns \
                 WHERE table_schema = DATABASE() AND table_name = ?",
            )
            .bind(m.table)
            .fetch_all(self.pool())
            .await
            .map_err(normalize)?;
            let have: HashSet<String> = cols.into_iter().map(|c| c.0.to_ascii_lowercase()).collect();
            for c in m.columns.iter().filter(|c| !have.contains(&c.name.to_ascii_lowercase())) {
                if c.auto {
                    return Err(OrmError::Unsupported(format!(
                        "表 {} 缺自增列 {}，Safe 模式不自动补（请人工处理）",
                        m.table, c.name
                    )));
                }
                let ddl = add_column_sql(m.table, c);
                self.exec_sql(&ddl).await?;
                executed.push(ddl);
            }

            let idx: Vec<(String,)> = sqlx::query_as(
                "SELECT DISTINCT CAST(index_name AS CHAR) FROM information_schema.statistics \
                 WHERE table_schema = DATABASE() AND table_name = ?",
            )
            .bind(m.table)
            .fetch_all(self.pool())
            .await
            .map_err(normalize)?;
            let have_idx: HashSet<String> = idx.into_iter().map(|i| i.0.to_ascii_lowercase()).collect();
            for c in m.columns.iter().filter(|c| c.unique || c.index) {
                let name = if c.unique { unique_index_name(m.table, c.name) } else { index_name(m.table, c.name) };
                if have_idx.contains(&name.to_ascii_lowercase()) {
                    continue;
                }
                let ddl = create_index_sql(m.table, c);
                self.exec_sql(&ddl).await?;
                executed.push(ddl);
            }
        }
        Ok(executed)
    }
}
