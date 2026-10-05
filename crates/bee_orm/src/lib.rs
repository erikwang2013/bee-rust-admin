// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! bee_orm：Beego 风格的异步 ORM，执行引擎基于 sqlx(MySQL)。

use sqlx::QueryBuilder;
use sqlx::mysql::{MySql, MySqlRow};

pub use bee_orm_macro::Model;

pub mod db;
pub mod error;
pub mod meta;
pub mod query;
pub mod syncdb;

pub use db::{Db, Tx};
pub use error::OrmError;
pub use meta::{ColumnMeta, ColumnType, ModelMeta, SyncdbMode};
pub use query::QuerySet;

/// 供 `#[derive(Model)]` 展开代码使用的内部重导出，不是公开 API。
#[doc(hidden)]
pub mod __private {
    pub use sqlx;
    pub use sqlx::QueryBuilder;
    pub use sqlx::Row;
    pub use sqlx::mysql::{MySql, MySqlRow};
}

/// 模型：元数据由 `#[derive(Model)]` 生成，CRUD 与 syncdb 都从这里取信息。
///
/// # 使用约束
///
/// 派生宏展开的代码引用 `bee_orm::…` 路径，因此使用 `#[derive(Model)]` 的模块里
/// `bee_orm` 必须在作用域内：要么本 crate 直接依赖 `bee_orm`（`use bee_orm::Model;`），
/// 要么走框架门面 `use bee_rust::bee_orm::{self, Model};`。
pub trait Model: Send + Sync + 'static {
    const META: ModelMeta;

    /// 从一行结果按列名映射出模型（宏生成）。
    fn from_row(row: &MySqlRow) -> Result<Self, sqlx::Error>
    where
        Self: Sized;

    /// 追加 INSERT 的绑定值，顺序与 META 中非自增列一致（宏生成）。
    fn bind_insert<'q>(&self, qb: QueryBuilder<'q, MySql>) -> QueryBuilder<'q, MySql>;

    /// 追加 UPDATE 的 `SET col = ?, ... WHERE pk = ?`（宏生成）。
    fn bind_update<'q>(&self, qb: QueryBuilder<'q, MySql>) -> QueryBuilder<'q, MySql>;

    /// 自增主键回填（宏在存在 `#[bee(auto)]` 字段时生成）。
    fn set_auto_pk(&mut self, _id: u64) {}
}
