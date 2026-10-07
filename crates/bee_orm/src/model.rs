// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! The [`Model`] trait: a struct mapped to a table, with default
//! `INSERT` / `UPDATE` / `DELETE` implementations built on [`Db`].
//!
//! The `#[derive(Model)]` macro from `bee_orm_macro` generates this trait for
//! a struct; hand-written impls work the same way.
//!
//! # Timestamps
//!
//! [`Model::auto_now_add_columns`] / [`Model::auto_now_columns`] name columns
//! written with the current unix time (seconds, stored as an integer):
//! `insert` fills both lists, `update` refreshes only `auto_now_columns`. The
//! value is injected by the trait — do **not** list those columns in
//! [`insert_values`](Model::insert_values) /
//! [`update_values`](Model::update_values) (the derive excludes them
//! automatically). One "now" is computed per statement. A field meant to hold
//! one must decode from an integer JSON number (`i64` / `Option<i64>`);
//! derives cannot see field types, so a wrong type fails at
//! [`from_row`](Model::from_row), not at compile time. After an insert the
//! in-memory struct still holds its old timestamp until re-read (Beego
//! semantics).
//!
//! # Soft delete
//!
//! [`Model::soft_delete_column`] names a boolean flag column. [`Model::delete`]
//! then flips the flag instead of issuing `DELETE` — deleting the same
//! instance twice affects 0 rows the second time — while [`Model::hard_delete`]
//! always issues the real `DELETE`. Both run the delete hooks. `QuerySet`
//! reads apply an implicit `flag = false` filter;
//! [`QuerySet::with_deleted`](crate::QuerySet::with_deleted) drops it.
//!
//! **NULL caveat:** the filter is `flag = ?` with `false`, and SQL
//! `NULL = false` is NULL — rows with NULL in the flag column are invisible to
//! default queries. Schema migrations must add the column `DEFAULT 0 NOT NULL`
//! (or backfill).
//!
//! # Hooks
//!
//! `before_insert` / `after_insert` (and the update / delete pairs) run
//! around the corresponding instance operations. `before_*` runs first; its
//! error aborts before any SQL runs. `after_*` runs after the statement
//! succeeded — its error is returned even though the write is already
//! committed (this layer has no transaction context). Hooks are instance
//! lifecycle: `QuerySet` operations and [`Model::insert_many`] do not run
//! them.

use async_trait::async_trait;

use crate::{Db, MAX_BIND_PARAMS, OrmError, Result, Row, Value};

/// The SQL dialect of a backend, as reported by
/// [`Db::dialect`](crate::Db::dialect). `#[non_exhaustive]`: match with a
/// wildcard arm.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dialect {
    Sqlite,
    Postgres,
    Mysql,
}

/// The canonical type of a column, before dialect rendering — see the
/// spelling-to-type table in `#[derive(Model)]`'s docs. `Raw` is a verbatim
/// string (`#[bee(sql_type = "…")]`), emitted as-is on every dialect.
///
/// `#[non_exhaustive]`: variants may be added; match with a wildcard arm.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SqlType {
    Int,
    BigInt,
    Real,
    Double,
    Bool,
    Text,
    Blob,
    /// A JSON document (`serde_json::Value`): `TEXT` on sqlite, `JSONB` on
    /// postgres, `JSON` on mysql.
    Json,
    /// A calendar date (`chrono::NaiveDate`): `TEXT` on sqlite.
    Date,
    /// A timezone-less timestamp (`chrono::NaiveDateTime`): `TEXT` on sqlite.
    DateTime,
    /// An instant (`chrono::DateTime<Utc>`): `TEXT` on sqlite.
    DateTimeTz,
    /// An arbitrary-precision decimal (`rust_decimal::Decimal`): `TEXT` on
    /// sqlite — a `DECIMAL` declaration would get NUMERIC affinity and turn
    /// the stored text into a `REAL`, losing the scale.
    Decimal,
    Raw(&'static str),
}

/// A column default, rendered as `DEFAULT <v>` (booleans render `0`/`1` on
/// sqlite/mysql and `false`/`true` on postgres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefaultValue {
    Bool(bool),
    Int(i64),
    /// Verbatim SQL, e.g. `CURRENT_TIMESTAMP`.
    Raw(&'static str),
}

/// A foreign-key target: the referenced table and its primary-key column.
///
/// A fn pair rather than strings: the derive resolves both from the target
/// type without instantiating anything, and DDL needs the pk column name.
#[derive(Debug, Clone, Copy)]
pub struct Reference {
    pub table: fn() -> &'static str,
    pub pk_column: fn() -> &'static str,
}

impl PartialEq for Reference {
    /// Compares the names the fns resolve to — comparing fn addresses is not
    /// meaningful (the same fn may live at different addresses per codegen
    /// unit, and distinct fns may share one).
    fn eq(&self, other: &Self) -> bool {
        (self.table)() == (other.table)() && (self.pk_column)() == (other.pk_column)()
    }
}

/// One column's schema metadata, as returned by [`Model::columns`].
///
/// [`migrate`](crate::migrate) renders these; the derive fills them from the
/// field spellings and attributes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnDef {
    pub name: &'static str,
    pub sql: SqlType,
    pub nullable: bool,
    pub primary_key: bool,
    /// `#[bee(auto)]`: the value is assigned by the database.
    pub auto_increment: bool,
    pub default: Option<DefaultValue>,
    /// `#[bee(fk = Target)]`; rendered as an inline `REFERENCES` clause.
    pub references: Option<Reference>,
}

/// One many-to-many relation of a model, as returned by [`Model::m2m`].
///
/// A join table has no `Model` of its own: [`m2m`](crate::m2m) reads it with
/// two `IN` queries and [`migrate`](crate::migrate) creates it alongside the
/// declaring model's table. Names come from the struct idents lowercased
/// (macro-time, so `#[bee(table = …)]` renames do not shift them).
#[derive(Debug, Clone, Copy)]
pub struct M2mDef {
    /// The join table.
    pub table: &'static str,
    /// The declaring model's column in the join table.
    pub local_column: &'static str,
    /// The target model's column in the join table.
    pub foreign_column: &'static str,
    /// Discovery key (matched against [`Model::table_name`]) and the
    /// `REFERENCES` target for DDL.
    pub target_table: fn() -> &'static str,
    /// The target's [`Model::columns`] (its pk name and [`SqlType`] for DDL).
    pub target_columns: fn() -> &'static [ColumnDef],
    /// The target's spelling as written in `#[bee(m2m(…))]`: last path
    /// segment, unrawed, case preserved. This is what the error hint splices
    /// into the attribute — never the (possibly renamed) table name.
    pub target_ident: &'static str,
}

impl PartialEq for M2mDef {
    /// Compares what the fns resolve to, never fn addresses (see
    /// [`Reference`]'s impl — same reason). `target_ident` is a plain `str`,
    /// so it compares directly.
    fn eq(&self, other: &Self) -> bool {
        self.table == other.table
            && self.local_column == other.local_column
            && self.foreign_column == other.foreign_column
            && (self.target_table)() == (other.target_table)()
            && (self.target_columns)() == (other.target_columns)()
            && self.target_ident == other.target_ident
    }
}

#[async_trait]
pub trait Model: Send + Sync + 'static {
    /// The table this model is mapped to.
    fn table_name() -> &'static str;

    /// The primary key column.
    fn pk_column() -> &'static str;

    /// Build an instance from one result row.
    ///
    /// `where Self: Sized` is required by `Result<Self>` (traits carry an
    /// implicit `Self: ?Sized`); every concrete model is `Sized`, so callers
    /// are unaffected.
    fn from_row(row: &Row) -> Result<Self>
    where
        Self: Sized;

    /// Column/value pairs for `INSERT` (skips database-assigned columns).
    ///
    /// Must not list columns named by
    /// [`auto_now_add_columns`](Model::auto_now_add_columns) /
    /// [`auto_now_columns`](Model::auto_now_columns): `insert` injects those.
    fn insert_values(&self) -> Vec<(&'static str, Value)>;

    /// The primary key value addressing this instance.
    fn pk_value(&self) -> Value;

    /// Column/value pairs for `UPDATE` (skips the primary key).
    ///
    /// Must not list columns named by
    /// [`auto_now_columns`](Model::auto_now_columns): `update` injects those.
    fn update_values(&self) -> Vec<(&'static str, Value)>;

    /// Columns written with "now" (unix seconds) on `insert` only.
    fn auto_now_add_columns() -> &'static [&'static str] {
        &[]
    }

    /// Columns written with "now" (unix seconds) on `insert` and `update`.
    fn auto_now_columns() -> &'static [&'static str] {
        &[]
    }

    /// The soft-delete flag column, when the model has one.
    fn soft_delete_column() -> Option<&'static str> {
        None
    }

    /// Schema metadata for [`migrate`](crate::migrate), in table order.
    ///
    /// Empty by default — the derive fills it; hand-written impls must
    /// implement it to use the migration helpers (they error on an empty
    /// list, naming the model).
    fn columns() -> &'static [ColumnDef] {
        &[]
    }

    /// Many-to-many relations declared on this model (`#[bee(m2m(Tag))]`).
    ///
    /// Empty by default; the derive emits it only when the struct declares
    /// one. The join tables are created by
    /// [`migrate::create_table`](crate::migrate::create_table) /
    /// [`migrate::sync`](crate::migrate::sync) — never by
    /// `add_missing_columns`, which is additive only.
    fn m2m() -> &'static [M2mDef] {
        &[]
    }

    /// Runs before `insert`; an error aborts before any SQL runs.
    async fn before_insert(&self) -> Result<()> {
        Ok(())
    }

    /// Runs after a successful `insert`; an error is returned even though the
    /// write is already committed.
    async fn after_insert(&self) -> Result<()> {
        Ok(())
    }

    /// Runs before `update`; an error aborts before any SQL runs.
    async fn before_update(&self) -> Result<()> {
        Ok(())
    }

    /// Runs after a successful `update`; an error is returned even though the
    /// write is already committed.
    async fn after_update(&self) -> Result<()> {
        Ok(())
    }

    /// Runs before `delete` / `hard_delete`; an error aborts before any SQL
    /// runs.
    async fn before_delete(&self) -> Result<()> {
        Ok(())
    }

    /// Runs after a successful `delete` / `hard_delete`; an error is returned
    /// even though the delete is already committed.
    async fn after_delete(&self) -> Result<()> {
        Ok(())
    }

    /// `INSERT` this instance; returns affected rows.
    ///
    /// Runs [`before_insert`](Model::before_insert), injects the timestamp
    /// columns, executes, then runs [`after_insert`](Model::after_insert).
    async fn insert<D: Db + ?Sized>(&self, db: &D) -> Result<u64> {
        self.before_insert().await?;
        let (sql, values) = insert_statement(self, batch_now::<Self>()?, "insert")?;
        let params: Vec<Value> = values.into_iter().map(|(_, value)| value).collect();
        let affected = db.execute(&sql, &params).await?;
        self.after_insert().await?;
        Ok(affected)
    }

    /// `INSERT` this instance and return it as stored.
    ///
    /// Like [`insert`](Model::insert), but reads the row back — through
    /// [`Db::insert_returning`] where the backend supports it, otherwise a
    /// select by [`pk_value`](Model::pk_value) — so database-assigned values
    /// (an auto-increment primary key, column defaults) are filled in. Runs
    /// the insert hooks; the read-back skips the soft-delete filter.
    /// [`OrmError::NotFound`] when the written row cannot be read back.
    async fn create<D: Db + ?Sized>(&self, db: &D) -> Result<Self>
    where
        Self: Sized,
    {
        self.before_insert().await?;
        let (sql, values) = insert_statement(self, batch_now::<Self>()?, "create")?;
        let has_pk = values.iter().any(|(column, _)| *column == Self::pk_column());
        let params: Vec<Value> = values.into_iter().map(|(_, value)| value).collect();
        let row = if has_pk {
            db.execute(&sql, &params).await?;
            None
        } else {
            db.insert_returning(Self::table_name(), Self::pk_column(), &sql, &params).await?
        };
        let row = match row {
            Some(row) => row,
            None => {
                let sql =
                    format!("SELECT * FROM {} WHERE {} = ?", Self::table_name(), Self::pk_column());
                let rows = db.query(&sql, &[self.pk_value()]).await?;
                rows.into_iter().next().ok_or(OrmError::NotFound)?
            }
        };
        self.after_insert().await?;
        Self::from_row(&row)
    }

    /// `UPDATE` this instance, addressed by primary key.
    ///
    /// Runs [`before_update`](Model::before_update), injects the
    /// [`auto_now_columns`](Model::auto_now_columns), executes, then runs
    /// [`after_update`](Model::after_update). The soft-delete flag is not
    /// consulted — updating a soft-deleted row works (also the restore path).
    async fn update<D: Db + ?Sized>(&self, db: &D) -> Result<u64> {
        self.before_update().await?;
        let values = update_row_values(self, batch_now::<Self>()?);
        if values.is_empty() {
            return Err(no_columns("update", Self::table_name()));
        }
        let assignments: Vec<String> =
            values.iter().map(|(column, _)| format!("{column} = ?")).collect();
        let sql = format!(
            "UPDATE {} SET {} WHERE {} = ?",
            Self::table_name(),
            assignments.join(", "),
            Self::pk_column()
        );
        let mut params: Vec<Value> = values.into_iter().map(|(_, value)| value).collect();
        params.push(self.pk_value());
        let affected = db.execute(&sql, &params).await?;
        self.after_update().await?;
        Ok(affected)
    }

    /// Delete this instance, addressed by primary key.
    ///
    /// On a model with a [`soft_delete_column`](Model::soft_delete_column)
    /// this is a soft `UPDATE` setting the flag; the extra `AND flag = ?`
    /// (false) term makes a second delete affect 0 rows. Otherwise a real
    /// `DELETE`. Runs the delete hooks.
    async fn delete<D: Db + ?Sized>(&self, db: &D) -> Result<u64> {
        self.before_delete().await?;
        let affected = match Self::soft_delete_column() {
            Some(column) => {
                let sql = format!(
                    "UPDATE {} SET {column} = ? WHERE {} = ? AND {column} = ?",
                    Self::table_name(),
                    Self::pk_column()
                );
                db.execute(&sql, &[Value::Bool(true), self.pk_value(), Value::Bool(false)]).await?
            }
            None => {
                let sql =
                    format!("DELETE FROM {} WHERE {} = ?", Self::table_name(), Self::pk_column());
                db.execute(&sql, &[self.pk_value()]).await?
            }
        };
        self.after_delete().await?;
        Ok(affected)
    }

    /// Always a real `DELETE` — the soft-delete flag is ignored. Runs the
    /// delete hooks.
    async fn hard_delete<D: Db + ?Sized>(&self, db: &D) -> Result<u64> {
        self.before_delete().await?;
        let sql = format!("DELETE FROM {} WHERE {} = ?", Self::table_name(), Self::pk_column());
        let affected = db.execute(&sql, &[self.pk_value()]).await?;
        self.after_delete().await?;
        Ok(affected)
    }

    /// Bulk `INSERT`: one statement per chunk of rows, at most 999 bound
    /// parameters per statement (`MAX_BIND_PARAMS`, the crate ceiling), one
    /// timestamp "now" for the whole batch. Hooks do not run.
    ///
    /// **Not transactional** — there is no transaction API at this layer. The
    /// first failing chunk aborts with the backend's error; rows in earlier
    /// chunks are already committed.
    async fn insert_many<D: Db + ?Sized>(db: &D, models: &[Self]) -> Result<u64>
    where
        Self: Sized,
    {
        if models.is_empty() {
            return Ok(0);
        }
        let now = batch_now::<Self>()?;
        let rows: Vec<Vec<(&'static str, Value)>> =
            models.iter().map(|model| insert_row_values(model, now)).collect();
        let columns: Vec<&str> = rows[0].iter().map(|(column, _)| *column).collect();
        if columns.is_empty() {
            return Err(no_columns("insert_many", Self::table_name()));
        }
        if columns.len() > MAX_BIND_PARAMS {
            return Err(OrmError::QueryError(format!(
                "insert_many: {} columns exceed the {MAX_BIND_PARAMS}-parameter batch limit",
                columns.len()
            )));
        }
        let chunk = MAX_BIND_PARAMS / columns.len();
        let tuple = format!("({})", vec!["?"; columns.len()].join(", "));
        let mut affected = 0u64;
        for group in rows.chunks(chunk) {
            let values_clause = vec![tuple.as_str(); group.len()].join(", ");
            let sql = format!(
                "INSERT INTO {} ({}) VALUES {values_clause}",
                Self::table_name(),
                columns.join(", ")
            );
            let mut params = Vec::with_capacity(columns.len() * group.len());
            for row in group {
                params.extend(row.iter().map(|(_, value)| value.clone()));
            }
            affected += db.execute(&sql, &params).await?;
        }
        Ok(affected)
    }
}

/// Unix seconds. The only error path is a clock set before 1970.
fn unix_now() -> Result<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .map_err(|_| OrmError::QueryError("system clock is set before the unix epoch".into()))
}

/// One "now" per statement when the model needs it, 0 otherwise.
fn batch_now<M: Model + ?Sized>() -> Result<i64> {
    if M::auto_now_add_columns().is_empty() && M::auto_now_columns().is_empty() {
        Ok(0)
    } else {
        unix_now()
    }
}

fn insert_row_values<M: Model + ?Sized>(model: &M, now: i64) -> Vec<(&'static str, Value)> {
    let mut values = model.insert_values();
    values.extend(M::auto_now_add_columns().iter().map(|c| (*c, Value::Int(now))));
    values.extend(M::auto_now_columns().iter().map(|c| (*c, Value::Int(now))));
    values
}

fn update_row_values<M: Model + ?Sized>(model: &M, now: i64) -> Vec<(&'static str, Value)> {
    let mut values = model.update_values();
    values.extend(M::auto_now_columns().iter().map(|c| (*c, Value::Int(now))));
    values
}

/// The `INSERT` statement and its named values (own columns plus the
/// timestamp injection); `op` names the caller in the no-columns error.
fn insert_statement<M: Model + ?Sized>(
    model: &M,
    now: i64,
    op: &str,
) -> Result<(String, Vec<(&'static str, Value)>)> {
    let values = insert_row_values(model, now);
    if values.is_empty() {
        return Err(no_columns(op, M::table_name()));
    }
    let columns: Vec<&str> = values.iter().map(|(column, _)| *column).collect();
    let placeholders = vec!["?"; values.len()].join(", ");
    let sql =
        format!("INSERT INTO {} ({}) VALUES ({placeholders})", M::table_name(), columns.join(", "));
    Ok((sql, values))
}

fn no_columns(op: &str, table: &str) -> OrmError {
    OrmError::QueryError(format!("{op} on `{table}`: no columns to write"))
}
