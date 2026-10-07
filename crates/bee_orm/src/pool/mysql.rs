// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! MySQL / TiDB pool (mysql_async's built-in pool).
//!
//! # Transactions
//!
//! Transactions must run on a connection held from `Pool::get()`: statements
//! sent through the `Pool` itself may each land on a different connection.
//! `CheckedConn::begin` / `commit` / `rollback` issue `BEGIN` / `COMMIT` /
//! `ROLLBACK` over the text protocol (`query_drop`) — MySQL 8 rejects those
//! statements in the prepared-statement protocol (error 1295). Dropping a
//! `CheckedConn` returns the connection to the pool;
//! the pool resets it on the next check-out (`reset_connection`, on by
//! default), which rolls back a leftover transaction. Always end a
//! transaction with `commit()` or `rollback()` — until then it holds locks.
//!
//! `Pool` has no `status()`: mysql_async exposes no pool statistics.
//!
//! ```no_run
//! # async fn demo() -> Result<(), bee_orm::OrmError> {
//! use bee_orm::pool::mysql::Pool;
//!
//! let pool = Pool::connect("mysql://user:pass@localhost:3306/app", 16)?;
//! let rows = pool
//!     .query("SELECT name FROM users WHERE age > ?", &[bee_orm::Value::from(18)])
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

use async_trait::async_trait;
use mysql_async::consts::ColumnType;
use mysql_async::prelude::Queryable;
use mysql_async::{Opts, OptsBuilder, Pool as MysqlPool, PoolConstraints, PoolOpts};
use serde_json::Value as Json;

use super::{Row, conn_err, query_err};
use crate::{Db, Dialect, OrmError, Result, Value};

/// ponytail: fixed 30s — mysql_async 0.34.2 has no acquire timeout; make it
/// configurable when a caller needs to tune it.
const GET_CONN_TIMEOUT: Duration = Duration::from_secs(30);

/// A cloneable handle to a mysql_async pool.
#[derive(Clone)]
pub struct Pool {
    inner: MysqlPool,
}

impl Pool {
    /// `dsn` is a URL, e.g. `"mysql://user:pass@host:3306/db"`.
    pub fn connect(dsn: &str, max_size: u32) -> Result<Self> {
        let opts = Opts::from_url(dsn).map_err(conn_err)?;
        let constraints = PoolConstraints::new(1, max_size.max(1) as usize)
            .ok_or_else(|| OrmError::ConnectionError("invalid pool size".into()))?;
        let opts = OptsBuilder::from_opts(opts)
            .pool_opts(PoolOpts::default().with_constraints(constraints));
        Ok(Self { inner: MysqlPool::new(opts) })
    }

    /// Check out one connection. Hold it for the length of a transaction
    /// (`begin` … `commit` / `rollback`). Waits at most 30 s for a slot.
    pub async fn get(&self) -> Result<CheckedConn> {
        let conn = tokio::time::timeout(GET_CONN_TIMEOUT, self.inner.get_conn())
            .await
            .map_err(|_| {
                OrmError::ConnectionError(
                    "pool exhausted: timed out waiting for a connection (mysql, 30s)".into(),
                )
            })?
            .map_err(conn_err)?;
        Ok(CheckedConn { conn })
    }

    pub async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        self.get().await?.query(sql, params).await
    }

    /// Returns the number of affected rows.
    pub async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        self.get().await?.execute(sql, params).await
    }
}

#[async_trait]
impl Db for Pool {
    fn dialect(&self) -> Option<Dialect> {
        Some(Dialect::Mysql)
    }

    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        Pool::query(self, sql, params).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        Pool::execute(self, sql, params).await
    }

    /// No `RETURNING` in MySQL: insert, then read the row back on the same
    /// connection. The key comes from `LAST_INSERT_ID()` only when the pk
    /// column is `AUTO_INCREMENT` — an explicit value leaves it untouched,
    /// and a pooled connection may carry an id from an earlier insert, so
    /// `None` is returned otherwise and `create` falls back to a pk select.
    async fn insert_returning(
        &self,
        table: &str,
        pk_column: &str,
        sql: &str,
        params: &[Value],
    ) -> Result<Option<Row>> {
        let mut conn = self.get().await?;
        conn.execute(sql, params).await?;
        let auto = "SELECT COUNT(*) AS n FROM information_schema.columns \
                    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? \
                    AND COLUMN_NAME = ? AND EXTRA LIKE '%auto_increment%'";
        let args = [Value::Text(table.to_owned()), Value::Text(pk_column.to_owned())];
        let found = conn
            .query(auto, &args)
            .await?
            .first()
            .and_then(|row| row.get("n"))
            .and_then(Json::as_i64)
            .unwrap_or(0);
        if found == 0 {
            return Ok(None);
        }
        let rows = conn.query("SELECT LAST_INSERT_ID() AS id", &[]).await?;
        let Some(pk) = rows.first().and_then(|row| row.get("id")).and_then(Json::as_i64) else {
            return Ok(None);
        };
        let select = format!("SELECT * FROM {table} WHERE {pk_column} = ? LIMIT 1");
        Ok(conn.query(&select, &[Value::Int(pk)]).await?.into_iter().next())
    }
}

/// A checked-out MySQL connection, returned to the pool on drop.
pub struct CheckedConn {
    conn: mysql_async::Conn,
}

impl CheckedConn {
    /// `&mut` because mysql_async's `Conn` methods require it.
    pub async fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let rows: Vec<mysql_async::Row> =
            self.conn.exec(sql, bind(params)).await.map_err(query_err)?;
        Ok(rows.into_iter().map(row_to_json).collect())
    }

    /// `&mut` because mysql_async's `Conn` methods require it.
    pub async fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64> {
        self.conn.exec_drop(sql, bind(params)).await.map_err(query_err)?;
        Ok(self.conn.affected_rows())
    }

    /// `BEGIN` on this connection. See the module docs on transactions.
    ///
    /// The three transaction-control statements go through the text protocol
    /// (`query_drop`), not `execute`: MySQL 8 rejects `BEGIN` / `COMMIT` /
    /// `ROLLBACK` in the prepared-statement protocol with error 1295.
    pub async fn begin(&mut self) -> Result<()> {
        self.conn.query_drop("BEGIN").await.map_err(query_err)
    }

    /// `COMMIT` the transaction started with [`CheckedConn::begin`].
    pub async fn commit(&mut self) -> Result<()> {
        self.conn.query_drop("COMMIT").await.map_err(query_err)
    }

    /// `ROLLBACK` the transaction started with [`CheckedConn::begin`].
    pub async fn rollback(&mut self) -> Result<()> {
        self.conn.query_drop("ROLLBACK").await.map_err(query_err)
    }
}

fn bind(params: &[Value]) -> Vec<mysql_async::Value> {
    params.iter().map(to_mysql).collect()
}

/// `Bool` → `Int(0|1)`, `Float` → `Double` (lossless), `Text` → `Bytes`,
/// `Json` → its serialized bytes (a `JSON` column accepts them). The
/// chrono / decimal variants convert through mysql_common's own impls: dates
/// as `Date` cells and decimals as their length-encoded text.
fn to_mysql(value: &Value) -> mysql_async::Value {
    use mysql_async::Value as My;
    match value {
        Value::Null => My::NULL,
        Value::Bool(v) => My::Int(i64::from(*v)),
        Value::Int(v) => My::Int(*v),
        Value::Float(v) => My::Double(*v),
        Value::Text(v) => My::Bytes(v.as_bytes().to_vec()),
        Value::Bytes(v) => My::Bytes(v.clone()),
        Value::Json(v) => My::Bytes(v.to_string().into_bytes()),
        #[cfg(feature = "chrono")]
        Value::Date(d) => My::from(*d),
        #[cfg(feature = "chrono")]
        Value::DateTime(dt) => My::from(*dt),
        // mysql has no timezone-aware column type: an instant binds as its UTC
        // naive form (a UTC session makes `timestamp` round-trip exactly).
        #[cfg(feature = "chrono")]
        Value::DateTimeUtc(dt) => My::from(dt.naive_utc()),
        #[cfg(feature = "rust_decimal")]
        Value::Decimal(d) => My::from(*d),
    }
}

fn row_to_json(row: mysql_async::Row) -> Row {
    // The column type rides along: a `V::Date` cell carries none of its own,
    // and `datetime` vs `timestamp` are only distinguishable by the column.
    let cols: Vec<(String, ColumnType)> =
        row.columns_ref().iter().map(|c| (c.name_str().to_string(), c.column_type())).collect();
    let mut out = Row::new();
    for ((name, column_type), v) in cols.into_iter().zip(row.unwrap()) {
        out.insert(name, value(v, column_type));
    }
    out
}

/// ponytail: bytes decode as UTF-8 text; date/time columns come back as null
/// unless the `chrono` feature is on, and then dispatched by the column's type.
fn value(
    v: mysql_async::Value,
    #[cfg_attr(not(feature = "chrono"), allow(unused_variables))] column_type: ColumnType,
) -> Json {
    use mysql_async::Value as V;
    match v {
        V::NULL => Json::Null,
        V::Int(i) => Json::from(i),
        V::UInt(u) => Json::from(u),
        V::Float(f) => Json::from(f),
        V::Double(d) => Json::from(d),
        V::Bytes(b) => Json::from(String::from_utf8_lossy(&b).into_owned()),
        #[cfg(feature = "chrono")]
        V::Date(y, mo, d, h, mi, s, micros) => date_cell(column_type, (y, mo, d, h, mi, s, micros)),
        #[cfg(not(feature = "chrono"))]
        V::Date(..) => Json::Null,
        // `NaiveTime` is out of scope: no backend has a consistent carrier.
        V::Time(..) => Json::Null,
    }
}

/// One `V::Date` cell as its serde spelling, dispatched by the column type:
/// `date` is `"YYYY-MM-DD"`, `datetime` the naive `"…T…"` form and `timestamp`
/// the same with a `Z` (mysql has no timezone-aware column, so a `timestamp`
/// cell is read as UTC). Serializing the chrono value keeps the cell symmetric
/// with `decode`; a zero or impossible date (mysql's `"0000-00-00"`) has no
/// chrono value and reads as `Json::Null`, as when the feature is off.
#[cfg(feature = "chrono")]
fn date_cell(
    column_type: ColumnType,
    (y, mo, d, h, mi, s, micros): (u16, u8, u8, u8, u8, u8, u32),
) -> Json {
    use chrono::{DateTime, NaiveDate, Utc};
    let Some(date) = NaiveDate::from_ymd_opt(y.into(), mo.into(), d.into()) else {
        return Json::Null;
    };
    if column_type == ColumnType::MYSQL_TYPE_DATE {
        return to_cell(date);
    }
    let Some(naive) = date.and_hms_micro_opt(h.into(), mi.into(), s.into(), micros) else {
        return Json::Null;
    };
    match column_type {
        ColumnType::MYSQL_TYPE_DATETIME => to_cell(naive),
        ColumnType::MYSQL_TYPE_TIMESTAMP => {
            to_cell(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
        }
        _ => Json::Null,
    }
}

/// A chrono value as its serde JSON cell (`Json::Null` when serialization
/// cannot represent it — same fallback as the read-side null convention).
#[cfg(feature = "chrono")]
fn to_cell<T: serde::Serialize>(value: T) -> Json {
    serde_json::to_value(value).unwrap_or(Json::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mysql_async::Value as My;

    #[test]
    fn bool_maps_to_zero_or_one() {
        assert_eq!(to_mysql(&Value::Bool(true)), My::Int(1));
        assert_eq!(to_mysql(&Value::Bool(false)), My::Int(0));
    }

    #[test]
    fn other_variants_map_one_to_one() {
        assert_eq!(to_mysql(&Value::Null), My::NULL);
        assert_eq!(to_mysql(&Value::Int(-7)), My::Int(-7));
        assert_eq!(to_mysql(&Value::Float(1.5)), My::Double(1.5));
        assert_eq!(to_mysql(&Value::Text("hi".into())), My::Bytes(b"hi".to_vec()));
        assert_eq!(to_mysql(&Value::Bytes(vec![1, 2])), My::Bytes(vec![1, 2]));
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn chrono_variants_bind_as_date_cells() {
        use chrono::{DateTime, NaiveDate, Utc};
        let date = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        let dt = date.and_hms_micro_opt(12, 34, 56, 123456).unwrap();
        assert_eq!(to_mysql(&Value::Date(date)), My::Date(2026, 10, 6, 0, 0, 0, 0));
        assert_eq!(to_mysql(&Value::DateTime(dt)), My::Date(2026, 10, 6, 12, 34, 56, 123456));
        // An instant binds as its UTC naive form.
        let utc = DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc);
        assert_eq!(to_mysql(&Value::DateTimeUtc(utc)), My::Date(2026, 10, 6, 12, 34, 56, 123456));
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn date_cells_decode_by_column_type() {
        use serde_json::json;
        let cell = My::Date(2026, 10, 6, 12, 34, 56, 123456);
        assert_eq!(value(cell.clone(), ColumnType::MYSQL_TYPE_DATE), json!("2026-10-06"));
        assert_eq!(
            value(cell.clone(), ColumnType::MYSQL_TYPE_DATETIME),
            json!("2026-10-06T12:34:56.123456")
        );
        assert_eq!(
            value(cell.clone(), ColumnType::MYSQL_TYPE_TIMESTAMP),
            json!("2026-10-06T12:34:56.123456Z")
        );
        // Whole seconds drop the fraction in both spellings.
        let whole = My::Date(2026, 10, 6, 12, 34, 56, 0);
        assert_eq!(
            value(whole.clone(), ColumnType::MYSQL_TYPE_DATETIME),
            json!("2026-10-06T12:34:56")
        );
        assert_eq!(value(whole, ColumnType::MYSQL_TYPE_TIMESTAMP), json!("2026-10-06T12:34:56Z"));
        // A zero date (mysql's "0000-00-00") has no chrono value: null.
        assert_eq!(value(My::Date(0, 0, 0, 0, 0, 0, 0), ColumnType::MYSQL_TYPE_DATE), Json::Null);
    }
}
