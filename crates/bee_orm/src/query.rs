// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::db::Db;
use crate::error::{OrmError, normalize, validate_ident};
use crate::Model;
use std::marker::PhantomData;

/// 流式 SQL 查询构造器。值一律参数化绑定；`to_sql()` 仅用于调试与测试。
pub struct QuerySet<T: Model> {
    table: String,
    filters: Vec<String>,
    params: Vec<String>,
    order_clauses: Vec<String>,
    limit_val: Option<usize>,
    offset_val: Option<usize>,
    _marker: PhantomData<T>,
}

// 手写 Clone：derive 会要求 `T: Clone`，但克隆只复制 SQL 片段，与 T 无关。
impl<T: Model> Clone for QuerySet<T> {
    fn clone(&self) -> Self {
        Self {
            table: self.table.clone(),
            filters: self.filters.clone(),
            params: self.params.clone(),
            order_clauses: self.order_clauses.clone(),
            limit_val: self.limit_val,
            offset_val: self.offset_val,
            _marker: PhantomData,
        }
    }
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

    /// 原样 WHERE 片段（调用方只传写死的常量；用户输入走 filter_eq/filter_in 等）。
    pub fn filter(mut self, condition: impl Into<String>) -> Self {
        self.filters.push(condition.into());
        self
    }

    /// `field = ?`
    pub fn filter_eq(mut self, field: impl Into<String>, value: impl ToString) -> Result<Self, OrmError> {
        let field = field.into();
        validate_ident(&field)?;
        self.filters.push(format!("{field} = ?"));
        self.params.push(value.to_string());
        Ok(self)
    }

    /// `field > ?`
    pub fn filter_gt(mut self, field: impl Into<String>, value: impl ToString) -> Result<Self, OrmError> {
        let field = field.into();
        validate_ident(&field)?;
        self.filters.push(format!("{field} > ?"));
        self.params.push(value.to_string());
        Ok(self)
    }

    /// `field < ?`
    pub fn filter_lt(mut self, field: impl Into<String>, value: impl ToString) -> Result<Self, OrmError> {
        let field = field.into();
        validate_ident(&field)?;
        self.filters.push(format!("{field} < ?"));
        self.params.push(value.to_string());
        Ok(self)
    }

    /// `field LIKE ?`，值两侧自动加 `%`。
    pub fn filter_contains(mut self, field: impl Into<String>, value: impl ToString) -> Result<Self, OrmError> {
        let field = field.into();
        validate_ident(&field)?;
        self.filters.push(format!("{field} LIKE ?"));
        self.params.push(format!("%{}%", value.to_string()));
        Ok(self)
    }

    /// `field IN (?, ?, ...)`；空集合生成恒假条件 `1 = 0`（语义正确且不报错）。
    pub fn filter_in<I, V>(mut self, field: impl Into<String>, values: I) -> Result<Self, OrmError>
    where
        I: IntoIterator<Item = V>,
        V: ToString,
    {
        let field = field.into();
        validate_ident(&field)?;
        let vals: Vec<String> = values.into_iter().map(|v| v.to_string()).collect();
        if vals.is_empty() {
            self.filters.push("1 = 0".into());
            return Ok(self);
        }
        let placeholders = vec!["?"; vals.len()].join(", ");
        self.filters.push(format!("{field} IN ({placeholders})"));
        self.params.extend(vals);
        Ok(self)
    }

    /// 原样 SQL 片段 + 绑定参数。片段必须是写死的常量；参数仍走绑定，不做字符串拼接。
    pub fn filter_raw<P: ToString>(mut self, sql: impl Into<String>, params: &[P]) -> Self {
        self.filters.push(sql.into());
        self.params.extend(params.iter().map(|p| p.to_string()));
        self
    }

    pub fn order_by(mut self, clause: impl Into<String>) -> Self {
        self.order_clauses.push(clause.into());
        self
    }

    pub fn limit(mut self, n: usize) -> Self {
        self.limit_val = Some(n);
        self
    }

    pub fn offset(mut self, n: usize) -> Self {
        self.offset_val = Some(n);
        self
    }

    /// 1 起始页码 + 每页条数（size 上限 500）。
    pub fn page(mut self, page: usize, size: usize) -> Self {
        let size = size.clamp(1, 500);
        self.limit_val = Some(size);
        self.offset_val = Some(page.saturating_sub(1).saturating_mul(size));
        self
    }

    /// 绑定参数（`?` 占位符按顺序对应）。
    pub fn params(&self) -> &[String] {
        &self.params
    }

    /// 构造 SELECT SQL（调试/测试用；执行走 fetch_*，值不被拼进 SQL）。
    pub fn to_sql(&self) -> String {
        let mut sql = format!("SELECT * FROM {}", self.table);
        self.push_where(&mut sql);
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

    /// COUNT 语句：丢掉 ORDER BY / LIMIT / OFFSET。
    pub fn count_sql(&self) -> String {
        let mut sql = format!("SELECT COUNT(*) FROM {}", self.table);
        self.push_where(&mut sql);
        sql
    }

    fn push_where(&self, sql: &mut String) {
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&self.filters.join(" AND "));
        }
    }

    /// 执行查询，返回全部行。
    pub async fn fetch_all(&self, db: &Db) -> Result<Vec<T>, OrmError> {
        let sql = self.to_sql();
        let mut q = sqlx::query(&sql);
        for p in &self.params {
            q = q.bind(p.clone());
        }
        let rows = q.fetch_all(db.pool()).await.map_err(normalize)?;
        rows.iter().map(T::from_row).collect::<Result<Vec<_>, _>>().map_err(normalize)
    }

    /// 执行查询，返回首行。
    pub async fn fetch_one(&self, db: &Db) -> Result<Option<T>, OrmError> {
        let sql = self.to_sql();
        let mut q = sqlx::query(&sql);
        for p in &self.params {
            q = q.bind(p.clone());
        }
        let row = q.fetch_optional(db.pool()).await.map_err(normalize)?;
        row.as_ref().map(T::from_row).transpose().map_err(normalize)
    }

    /// 计数。MySQL 的 `COUNT(*)` 返回有符号 BIGINT，直接解码 u64 会被 sqlx 拒绝
    /// （mismatched types），故按 i64 解码后再转（行数非负）。
    pub async fn count(&self, db: &Db) -> Result<u64, OrmError> {
        let sql = self.count_sql();
        let mut q = sqlx::query_scalar::<_, i64>(&sql);
        for p in &self.params {
            q = q.bind(p.clone());
        }
        q.fetch_one(db.pool()).await.map_err(normalize).map(|n| n as u64)
    }

    /// 分页：返回（当前页数据, 总数）。`page` 为 1 起始页码。
    pub async fn fetch_page(&self, db: &Db, page: usize, size: usize) -> Result<(Vec<T>, u64), OrmError> {
        let total = self.count(db).await?;
        let rows = self.clone().page(page, size).fetch_all(db).await?;
        Ok((rows, total))
    }
}
