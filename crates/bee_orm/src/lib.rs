// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! bee_orm：Beego 风格的异步 ORM，执行引擎基于 sqlx(MySQL)。

use std::marker::PhantomData;

use sqlx::QueryBuilder;
use sqlx::mysql::{MySql, MySqlRow};

pub use bee_orm_macro::Model;

pub mod error;
pub mod meta;

pub use error::OrmError;
pub use meta::{ColumnMeta, ColumnType, ModelMeta, SyncdbMode};

/// 供 `#[derive(Model)]` 展开代码使用的内部重导出，不是公开 API。
#[doc(hidden)]
pub mod __private {
    pub use sqlx;
    pub use sqlx::QueryBuilder;
    pub use sqlx::Row;
    pub use sqlx::mysql::{MySql, MySqlRow};
}

/// 模型：元数据由 `#[derive(Model)]` 生成，CRUD 与 syncdb 都从这里取信息。
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

/// A fluent SQL query builder for a model type `T`.
pub struct QuerySet<T: Model> {
    table: String,
    filters: Vec<String>,
    params: Vec<String>,
    order_clauses: Vec<String>,
    limit_val: Option<usize>,
    offset_val: Option<usize>,
    _marker: PhantomData<T>,
}

impl<T: Model> QuerySet<T> {
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            filters: Vec::new(),
            params: Vec::new(),
            order_clauses: Vec::new(),
            limit_val: None,
            offset_val: None,
            _marker: PhantomData,
        }
    }

    /// Add a raw WHERE condition (e.g. `"age > 18"`).
    ///
    /// **Warning:** The string is concatenated into SQL verbatim. Only use it
    /// with trusted constants; use `filter_eq` / `filter_gt` / `filter_lt` /
    /// `filter_contains` for any value that comes from user input.
    pub fn filter(mut self, condition: impl Into<String>) -> Self {
        self.filters.push(condition.into());
        self
    }

    /// Add a parameterised equality condition: `field = ?`.
    ///
    /// The value is never concatenated into SQL; it is accumulated in
    /// [`QuerySet::params`], which the backend driver must bind to the `?`
    /// placeholders (in order) before execution.
    ///
    /// The field name is validated against `[A-Za-z_][A-Za-z0-9_]*`; anything
    /// else (e.g. user input with spaces or quotes) is rejected with
    /// [`OrmError::InvalidField`] instead of being spliced into SQL.
    pub fn filter_eq(
        mut self,
        field: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, OrmError> {
        let field = field.into();
        validate_field(&field)?;
        self.filters.push(format!("{field} = ?"));
        self.params.push(value.into());
        Ok(self)
    }

    /// Add a parameterised greater-than condition: `field > ?`.
    pub fn filter_gt(
        mut self,
        field: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, OrmError> {
        let field = field.into();
        validate_field(&field)?;
        self.filters.push(format!("{field} > ?"));
        self.params.push(value.into());
        Ok(self)
    }

    /// Add a parameterised less-than condition: `field < ?`.
    pub fn filter_lt(
        mut self,
        field: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, OrmError> {
        let field = field.into();
        validate_field(&field)?;
        self.filters.push(format!("{field} < ?"));
        self.params.push(value.into());
        Ok(self)
    }

    /// Add a parameterised substring condition: `field LIKE ?`, with the value
    /// wrapped in `%...%` wildcards.
    pub fn filter_contains(
        mut self,
        field: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, OrmError> {
        let field = field.into();
        validate_field(&field)?;
        self.filters.push(format!("{field} LIKE ?"));
        self.params.push(format!("%{}%", value.into()));
        Ok(self)
    }

    /// Bound parameters for the `?` placeholders in SQL, in positional order.
    ///
    /// Each `filter_eq` / `filter_gt` / `filter_lt` / `filter_contains` call
    /// appends exactly one parameter. Raw `filter` strings carry no parameter.
    pub fn params(&self) -> &[String] {
        &self.params
    }

    /// Add an ORDER BY clause (e.g. `"id DESC"`).
    pub fn order_by(mut self, clause: impl Into<String>) -> Self {
        self.order_clauses.push(clause.into());
        self
    }

    /// Set the LIMIT value.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit_val = Some(n);
        self
    }

    /// Set the OFFSET value.
    pub fn offset(mut self, n: usize) -> Self {
        self.offset_val = Some(n);
        self
    }

    /// Build the SQL string for this query (debugging and tests only).
    ///
    /// **Warning:** This method concatenates raw `filter` and `order_by`
    /// strings directly into SQL. In production, NEVER execute this string
    /// with values embedded — it is an injection path. Production queries
    /// MUST use the parameterised API (`filter_eq` / `filter_gt` / `filter_lt`
    /// / `filter_contains`) and bind [`QuerySet::params`] to the `?`
    /// placeholders, in order, through the driver's prepared-statement
    /// interface.
    pub fn to_sql(&self) -> String {
        let capacity = 16
            + self.table.len()
            + self.filters.iter().map(|f| f.len() + 5).sum::<usize>()
            + self.order_clauses.iter().map(|o| o.len() + 2).sum::<usize>()
            + 20;
        let mut sql = String::with_capacity(capacity);

        sql.push_str("SELECT * FROM ");
        sql.push_str(&self.table);

        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&self.filters.join(" AND "));
        }

        if !self.order_clauses.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&self.order_clauses.join(", "));
        }

        if let Some(limit) = self.limit_val {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        if let Some(offset) = self.offset_val {
            sql.push_str(&format!(" OFFSET {offset}"));
        }

        sql
    }
}

/// Column names may only be `[A-Za-z_][A-Za-z0-9_]*`; anything else could
/// splice SQL out of the quoted identifier context.
fn validate_field(field: &str) -> Result<(), OrmError> {
    let mut chars = field.chars();
    let valid = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid { Ok(()) } else { Err(OrmError::InvalidField(field.to_string())) }
}
