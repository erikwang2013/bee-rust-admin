// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 连接表（`admin_role` / `role_menu` / `role_dept` / `notice_read`）的读写。
//!
//! 上游 `bee_orm` 表达关联的范式是「在模型上用 `#[bee(m2m(...))]` 声明」，由框架
//! 生成连接表并成套提供读写；而本项目的连接表是**裸 DDL 建的**（复合主键，`migrate`
//! 只建单列主键、表达不了），并且按表名直接读写的地方有近二十处。
//! 与其把整套关联改成声明式（结构性重构），这里用框架支持的原子能力
//! （带参裸 SQL + `Pool::get()` 的事务）实现同样语义，做成 `Pool` 的扩展方法，
//! 好让既有调用点的写法一个字都不用改。
//!
//! **表名与列名是拼进 SQL 的**——它们全部来自本文件调用方的字面量常量，不是用户输入；
//! 值一律走绑定参数。

use bee_orm::pool::mysql::Pool;
use bee_orm::{OrmError, Value};

pub trait RelationsExt {
    /// 取某一行在连接表里关联到的目标 id 列表。
    async fn get_relations(
        &self,
        table: &str,
        key: (&str, i64),
        target: &str,
    ) -> Result<Vec<i64>, OrmError>;

    /// 全量覆盖某一行在连接表里的关联：先删旧的，再逐条插新的。
    /// **在一个事务里**——中途失败不能留下「旧的删了、新的没插上」的空档。
    async fn set_relations(
        &self,
        table: &str,
        key: (&str, i64),
        target: &str,
        ids: &[i64],
    ) -> Result<(), OrmError>;

    /// 删除某一行在连接表里的全部关联，返回删除行数。
    async fn del_relations(&self, table: &str, key: &str, val: i64) -> Result<u64, OrmError>;

    /// 数某一行被引用了多少次（删角色/菜单前的引用校验）。
    async fn count_refs(&self, table: &str, col: &str, val: i64) -> Result<i64, OrmError>;
}

impl RelationsExt for Pool {
    async fn get_relations(
        &self,
        table: &str,
        (key_col, key_val): (&str, i64),
        target: &str,
    ) -> Result<Vec<i64>, OrmError> {
        let sql = format!("SELECT {target} FROM {table} WHERE {key_col} = ?");
        let rows = self.query(&sql, &[Value::from(key_val)]).await?;
        Ok(rows
            .iter()
            .filter_map(|r| r.get(target).and_then(|v| v.as_i64()))
            .collect())
    }

    async fn set_relations(
        &self,
        table: &str,
        (key_col, key_val): (&str, i64),
        target: &str,
        ids: &[i64],
    ) -> Result<(), OrmError> {
        let mut conn = self.get().await?;
        conn.begin().await?;
        let result = async {
            conn.execute(
                &format!("DELETE FROM {table} WHERE {key_col} = ?"),
                &[Value::from(key_val)],
            )
            .await?;
            let insert = format!("INSERT INTO {table} ({key_col}, {target}) VALUES (?, ?)");
            for id in ids {
                conn.execute(&insert, &[Value::from(key_val), Value::from(*id)])
                    .await?;
            }
            Ok::<(), OrmError>(())
        }
        .await;
        match result {
            Ok(()) => {
                conn.commit().await?;
                Ok(())
            }
            Err(e) => {
                // 回滚失败也把原始错误抛出去：那是真正要查的那个
                let _ = conn.rollback().await;
                Err(e)
            }
        }
    }

    async fn del_relations(&self, table: &str, key: &str, val: i64) -> Result<u64, OrmError> {
        self.execute(
            &format!("DELETE FROM {table} WHERE {key} = ?"),
            &[Value::from(val)],
        )
        .await
    }

    async fn count_refs(&self, table: &str, col: &str, val: i64) -> Result<i64, OrmError> {
        let sql = format!("SELECT COUNT(*) AS n FROM {table} WHERE {col} = ?");
        let rows = self.query(&sql, &[Value::from(val)]).await?;
        Ok(rows
            .first()
            .and_then(|r| r.get("n"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0))
    }
}
