// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! [`QuerySet`]: a fluent SQL query builder for a model type `T`, plus
//! execution of the built statement against any [`Db`].

use std::marker::PhantomData;

use crate::{Db, Model, OrmError, Result, Row, Value};

/// A fluent SQL query builder for a model type `T`.
///
/// For a model with a soft-delete flag column, reads, updates, deletes and
/// aggregates carry an implicit `flag = false` filter as the first WHERE
/// term; [`QuerySet::with_deleted`] drops it and [`QuerySet::hard_delete`]
/// issues the real `DELETE`.
pub struct QuerySet<T: Model> {
    table: String,
    filters: Vec<String>,
    params: Vec<Value>,
    order_clauses: Vec<String>,
    limit_val: Option<usize>,
    offset_val: Option<usize>,
    /// `(column, active flag value)` from `T::soft_delete_column()`.
    soft_filter: Option<(&'static str, Value)>,
    include_deleted: bool,
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
            soft_filter: T::soft_delete_column().map(|column| (column, Value::Bool(false))),
            include_deleted: false,
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
    pub fn filter_eq(mut self, field: impl Into<String>, value: impl Into<Value>) -> Result<Self> {
        let field = field.into();
        validate_field(&field)?;
        self.filters.push(format!("{field} = ?"));
        self.params.push(value.into());
        Ok(self)
    }

    /// Add a parameterised greater-than condition: `field > ?`.
    pub fn filter_gt(mut self, field: impl Into<String>, value: impl Into<Value>) -> Result<Self> {
        let field = field.into();
        validate_field(&field)?;
        self.filters.push(format!("{field} > ?"));
        self.params.push(value.into());
        Ok(self)
    }

    /// Add a parameterised less-than condition: `field < ?`.
    pub fn filter_lt(mut self, field: impl Into<String>, value: impl Into<Value>) -> Result<Self> {
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
    ) -> Result<Self> {
        let field = field.into();
        validate_field(&field)?;
        self.filters.push(format!("{field} LIKE ?"));
        self.params.push(format!("%{}%", value.into()).into());
        Ok(self)
    }

    /// Add a parameterised set condition: `field IN (?, ?, …)`, one bound
    /// parameter per value in positional order.
    ///
    /// An empty `values` slice is an [`OrmError::QueryError`] — a silent
    /// nothing-matching `IN ()` is the classic way such bugs ship; callers
    /// with possibly-empty input skip the call instead. The field name is
    /// validated like the other parameterised filters.
    pub fn filter_in(mut self, field: impl Into<String>, values: &[Value]) -> Result<Self> {
        let field = field.into();
        validate_field(&field)?;
        if values.is_empty() {
            return Err(OrmError::QueryError("filter_in: empty value list".into()));
        }
        let placeholders = vec!["?"; values.len()].join(", ");
        self.filters.push(format!("{field} IN ({placeholders})"));
        self.params.extend_from_slice(values);
        Ok(self)
    }

    /// Bound parameters for the `?` placeholders in SQL, in positional order.
    ///
    /// Each `filter_eq` / `filter_gt` / `filter_lt` / `filter_contains` call
    /// appends exactly one parameter. Raw `filter` strings carry no parameter.
    /// On a soft-delete model the implicit `flag = ?` term is not part of this
    /// slice: the execution methods bind it themselves (a manual `to_sql()` +
    /// `params()` pairing must prepend `Value::Bool(false)`, or use
    /// [`with_deleted`](Self::with_deleted)).
    pub fn params(&self) -> &[Value] {
        &self.params
    }

    /// Include soft-deleted rows: drops the implicit `flag = false` filter
    /// from reads, `update`, `delete` and the aggregates.
    pub fn with_deleted(mut self) -> Self {
        self.include_deleted = true;
        self
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
        self.select_sql(None)
    }

    /// Run the query and decode every row into `T`.
    pub async fn all<D: Db + ?Sized>(&self, db: &D) -> Result<Vec<T>> {
        self.rows(db).await?.iter().map(T::from_row).collect()
    }

    /// Run the SELECT and return the raw rows (`all` decodes them).
    pub(crate) async fn rows<D: Db + ?Sized>(&self, db: &D) -> Result<Vec<Row>> {
        db.query(&self.select_sql(None), &self.active_params()).await
    }

    /// `LIMIT 1`; decodes the first row, if any. Overrides any earlier `limit()`.
    pub async fn one<D: Db + ?Sized>(&self, db: &D) -> Result<Option<T>> {
        let rows = db.query(&self.select_sql(Some(1)), &self.active_params()).await?;
        rows.first().map(T::from_row).transpose()
    }

    /// `SELECT COUNT(*) AS count` with the WHERE clause only — ORDER BY / LIMIT
    /// / OFFSET are ignored.
    pub async fn count<D: Db + ?Sized>(&self, db: &D) -> Result<i64> {
        let sql =
            format!("SELECT COUNT(*) AS count FROM {}{}", self.table, self.active_where_sql());
        let rows = db.query(&sql, &self.active_params()).await?;
        rows.first()
            .and_then(|row| row.get("count"))
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| OrmError::QueryError("count: unexpected result shape".into()))
    }

    /// Whether at least one row matches (`SELECT 1 … LIMIT 1`; no row is
    /// decoded).
    pub async fn exists<D: Db + ?Sized>(&self, db: &D) -> Result<bool> {
        let sql = format!("SELECT 1 FROM {}{} LIMIT 1", self.table, self.active_where_sql());
        Ok(!db.query(&sql, &self.active_params()).await?.is_empty())
    }

    /// `UPDATE` the matching rows. SET parameters are bound before WHERE
    /// parameters.
    pub async fn update<D: Db + ?Sized>(&self, db: &D, sets: &[(&str, Value)]) -> Result<u64> {
        if sets.is_empty() {
            return Err(OrmError::QueryError("update: no columns to set".into()));
        }
        let mut assignments = Vec::with_capacity(sets.len());
        let mut params: Vec<Value> = Vec::with_capacity(sets.len() + self.params.len() + 1);
        for (field, value) in sets {
            validate_field(field)?;
            assignments.push(format!("{field} = ?"));
            params.push(value.clone());
        }
        params.extend_from_slice(&self.active_params());
        let sql = format!(
            "UPDATE {} SET {}{}",
            self.table,
            assignments.join(", "),
            self.active_where_sql()
        );
        db.execute(&sql, &params).await
    }

    /// Delete the matching rows. On a soft-delete model this is the flag
    /// `UPDATE` (the active filter keeps already-deleted rows out; use
    /// [`hard_delete`](Self::hard_delete) for a real `DELETE`), otherwise a
    /// plain `DELETE`. With no filters this affects every active row (soft
    /// model) or every row.
    pub async fn delete<D: Db + ?Sized>(&self, db: &D) -> Result<u64> {
        match self.soft_filter.as_ref() {
            Some((column, _)) => {
                let column = *column;
                let mut params: Vec<Value> = Vec::with_capacity(self.params.len() + 2);
                params.push(Value::Bool(true));
                params.extend_from_slice(&self.active_params());
                let sql =
                    format!("UPDATE {} SET {column} = ?{}", self.table, self.active_where_sql());
                db.execute(&sql, &params).await
            }
            None => {
                let sql = format!("DELETE FROM {}{}", self.table, self.active_where_sql());
                db.execute(&sql, &self.active_params()).await
            }
        }
    }

    /// Plain `DELETE` under the active WHERE — the escape hatch for soft
    /// models: purges matching active rows, and with
    /// [`with_deleted`](Self::with_deleted) the soft-deleted ones too.
    pub async fn hard_delete<D: Db + ?Sized>(&self, db: &D) -> Result<u64> {
        let sql = format!("DELETE FROM {}{}", self.table, self.active_where_sql());
        db.execute(&sql, &self.active_params()).await
    }

    /// `SELECT SUM(field) AS value …`; `None` when no row matches (SQL NULL).
    pub async fn sum<D: Db + ?Sized>(
        &self,
        db: &D,
        field: impl Into<String>,
    ) -> Result<Option<f64>> {
        self.aggregate_f64(db, "SUM", field).await
    }

    /// `SELECT AVG(field) AS value …`; `None` when no row matches (SQL NULL).
    pub async fn avg<D: Db + ?Sized>(
        &self,
        db: &D,
        field: impl Into<String>,
    ) -> Result<Option<f64>> {
        self.aggregate_f64(db, "AVG", field).await
    }

    /// `SELECT MIN(field) AS value …` — the raw JSON value (text and date
    /// columns are legitimate); `None` when no row matches (SQL NULL).
    pub async fn min<D: Db + ?Sized>(
        &self,
        db: &D,
        field: impl Into<String>,
    ) -> Result<Option<serde_json::Value>> {
        self.aggregate_raw(db, "MIN", field).await
    }

    /// `SELECT MAX(field) AS value …` — the raw JSON value; `None` when no
    /// row matches (SQL NULL).
    pub async fn max<D: Db + ?Sized>(
        &self,
        db: &D,
        field: impl Into<String>,
    ) -> Result<Option<serde_json::Value>> {
        self.aggregate_raw(db, "MAX", field).await
    }

    /// `SUM` / `AVG`: numeric result, `None` on SQL NULL.
    async fn aggregate_f64<D: Db + ?Sized>(
        &self,
        db: &D,
        function: &str,
        field: impl Into<String>,
    ) -> Result<Option<f64>> {
        let field = field.into();
        validate_field(&field)?;
        let rows = db.query(&self.aggregate_sql(function, &field), &self.active_params()).await?;
        let value = aggregate_value(&rows, function)?;
        match value {
            serde_json::Value::Null => Ok(None),
            other => json_to_f64(other).map(Some).ok_or_else(|| {
                OrmError::QueryError(format!("{}: non-numeric result", function.to_lowercase()))
            }),
        }
    }

    /// `MIN` / `MAX`: whatever JSON the column yields, `None` on SQL NULL.
    async fn aggregate_raw<D: Db + ?Sized>(
        &self,
        db: &D,
        function: &str,
        field: impl Into<String>,
    ) -> Result<Option<serde_json::Value>> {
        let field = field.into();
        validate_field(&field)?;
        let rows = db.query(&self.aggregate_sql(function, &field), &self.active_params()).await?;
        let value = aggregate_value(&rows, function)?;
        Ok(if value.is_null() { None } else { Some(value.clone()) })
    }

    /// `SELECT {function}({field}) AS value FROM {table}{active_where}`.
    fn aggregate_sql(&self, function: &str, field: &str) -> String {
        format!(
            "SELECT {function}({field}) AS value FROM {}{}",
            self.table,
            self.active_where_sql()
        )
    }

    /// Build the SELECT statement; `limit_override` wins over `limit()`.
    fn select_sql(&self, limit_override: Option<usize>) -> String {
        let limit = limit_override.or(self.limit_val);
        let capacity = 16
            + self.table.len()
            + self.filters.iter().map(|f| f.len() + 5).sum::<usize>()
            + self.order_clauses.iter().map(|o| o.len() + 2).sum::<usize>()
            + 20;
        let mut sql = String::with_capacity(capacity);

        sql.push_str("SELECT * FROM ");
        sql.push_str(&self.table);
        sql.push_str(&self.active_where_sql());

        if !self.order_clauses.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&self.order_clauses.join(", "));
        }

        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        if let Some(offset) = self.offset_val {
            sql.push_str(&format!(" OFFSET {offset}"));
        }

        sql
    }

    /// The active soft-delete term as `(column, flag_value)`, when the model
    /// has a soft-delete column and `with_deleted()` has not been called.
    fn soft_term(&self) -> Option<(&'static str, &Value)> {
        if self.include_deleted {
            return None;
        }
        self.soft_filter.as_ref().map(|(column, value)| (*column, value))
    }

    /// `""` or `" WHERE flag = ? AND a AND b"` — the flag term first while it
    /// is active.
    fn active_where_sql(&self) -> String {
        match self.soft_term() {
            Some((column, _)) if self.filters.is_empty() => format!(" WHERE {column} = ?"),
            Some((column, _)) => {
                format!(" WHERE {column} = ? AND {}", self.filters.join(" AND "))
            }
            None => self.where_sql(),
        }
    }

    /// The flag parameter first (it is the first `?`), then the user params.
    fn active_params(&self) -> Vec<Value> {
        let mut params = Vec::with_capacity(self.params.len() + 1);
        if let Some((_, value)) = self.soft_term() {
            params.push(value.clone());
        }
        params.extend_from_slice(&self.params);
        params
    }

    /// `" WHERE …"` when there are filters, empty string otherwise.
    fn where_sql(&self) -> String {
        if self.filters.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.filters.join(" AND "))
        }
    }
}

/// Column names may only be `[A-Za-z_][A-Za-z0-9_]*`; anything else could
/// splice SQL out of the quoted identifier context.
fn validate_field(field: &str) -> Result<()> {
    let mut chars = field.chars();
    let valid = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid { Ok(()) } else { Err(OrmError::InvalidField(field.to_string())) }
}

/// Numbers pass through; strings parse (pg returns NUMERIC as a string).
fn json_to_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn aggregate_value<'a>(rows: &'a [Row], function: &str) -> Result<&'a serde_json::Value> {
    rows.first().and_then(|row| row.get("value")).ok_or_else(|| {
        OrmError::QueryError(format!("{}: unexpected result shape", function.to_lowercase()))
    })
}
