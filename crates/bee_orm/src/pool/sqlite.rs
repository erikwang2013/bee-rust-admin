// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! SQLite pool (r2d2 + rusqlite).
//!
//! The driver is synchronous, so the pool-level `query` / `execute` run on
//! `spawn_blocking` and are safe to `.await` from async handlers. A checked
//! out [`CheckedConn`] is deliberately synchronous — call it inside
//! `tokio::task::spawn_blocking` if you use it from async code.
//!
//! # Transactions
//!
//! Transactions must run on a connection held from `Pool::get()`: statements
//! sent through the `Pool` itself may each land on a different connection.
//! `CheckedConn::begin` / `commit` / `rollback` issue `BEGIN` / `COMMIT` /
//! `ROLLBACK`. A `CheckedConn` dropped mid-transaction runs a blocking
//! `ROLLBACK` before the connection returns to the pool; only transactions
//! started with `begin()` are tracked. Always end a transaction with
//! `commit()` or `rollback()`.
//!
//! ```no_run
//! # async fn demo() -> Result<(), bee_orm::OrmError> {
//! use bee_orm::pool::sqlite::Pool;
//!
//! let pool = Pool::connect("app.db", 8)?;
//! let rows = pool
//!     .query("SELECT name FROM users WHERE age > ?", &[bee_orm::Value::from(18)])
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params_from_iter;

use super::{Row, conn_err, query_err};
use crate::{Db, Dialect, Result, Value};

/// A cloneable handle to an r2d2 SQLite pool.
#[derive(Clone)]
pub struct Pool {
    inner: r2d2::Pool<SqliteConnectionManager>,
}

impl Pool {
    /// `dsn` is a file path, or `":memory:"` for an in-memory database.
    ///
    /// In-memory databases are clamped to a single connection: every SQLite
    /// connection to `:memory:` opens its own private database, so a larger
    /// pool would silently spread writes across unrelated databases.
    pub fn connect(dsn: &str, max_size: u32) -> Result<Self> {
        let (manager, max_size) = if dsn == ":memory:" {
            (SqliteConnectionManager::memory(), 1)
        } else {
            (SqliteConnectionManager::file(dsn), max_size.max(1))
        };
        let inner = r2d2::Pool::builder().max_size(max_size).build(manager).map_err(conn_err)?;
        Ok(Self { inner })
    }

    /// Check out one connection. Hold it for the length of a transaction
    /// (`begin` … `commit` / `rollback`).
    pub fn get(&self) -> Result<CheckedConn> {
        let conn = self.inner.get().map_err(conn_err)?;
        Ok(CheckedConn { conn, in_transaction: AtomicBool::new(false) })
    }

    /// Run a query on a pooled connection (on `spawn_blocking`).
    pub async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let (pool, sql, params) = (self.clone(), sql.to_owned(), params.to_vec());
        tokio::task::spawn_blocking(move || pool.get()?.query(&sql, &params))
            .await
            .map_err(conn_err)?
    }

    /// Run a statement on a pooled connection (on `spawn_blocking`), returning
    /// the number of affected rows.
    pub async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        let (pool, sql, params) = (self.clone(), sql.to_owned(), params.to_vec());
        tokio::task::spawn_blocking(move || pool.get()?.execute(&sql, &params))
            .await
            .map_err(conn_err)?
    }

    /// Pool statistics: `connections` (open), `idle_connections`.
    pub fn status(&self) -> r2d2::State {
        self.inner.state()
    }
}

#[async_trait]
impl Db for Pool {
    fn dialect(&self) -> Option<Dialect> {
        Some(Dialect::Sqlite)
    }

    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        Pool::query(self, sql, params).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        Pool::execute(self, sql, params).await
    }

    /// `INSERT … RETURNING *` — the stored row, read on the one connection
    /// that ran the statement (SQLite has supported it since 3.35).
    async fn insert_returning(
        &self,
        _table: &str,
        _pk_column: &str,
        sql: &str,
        params: &[Value],
    ) -> Result<Option<Row>> {
        let sql = format!("{sql} RETURNING *");
        Ok(Pool::query(self, &sql, params).await?.into_iter().next())
    }
}

/// A checked-out SQLite connection, returned to the pool on drop.
pub struct CheckedConn {
    conn: r2d2::PooledConnection<SqliteConnectionManager>,
    in_transaction: AtomicBool,
}

impl CheckedConn {
    pub fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let mut stmt = self.conn.prepare(sql).map_err(query_err)?;
        let names: Vec<String> = stmt.column_names().into_iter().map(|n| n.to_string()).collect();
        let mut rows = stmt.query(params_from_iter(params)).map_err(query_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(query_err)? {
            let mut map = Row::new();
            for (i, name) in names.iter().enumerate() {
                map.insert(name.clone(), value(row.get_ref(i).map_err(query_err)?));
            }
            out.push(map);
        }
        Ok(out)
    }

    pub fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        let affected = self.conn.execute(sql, params_from_iter(params)).map_err(query_err)?;
        Ok(affected as u64)
    }

    /// `BEGIN` on this connection. See the module docs on transactions.
    pub fn begin(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN").map_err(query_err)?;
        self.in_transaction.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// `COMMIT` the transaction started with [`CheckedConn::begin`].
    pub fn commit(&self) -> Result<()> {
        self.conn.execute_batch("COMMIT").map_err(query_err)?;
        self.in_transaction.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// `ROLLBACK` the transaction started with [`CheckedConn::begin`].
    pub fn rollback(&self) -> Result<()> {
        self.conn.execute_batch("ROLLBACK").map_err(query_err)?;
        self.in_transaction.store(false, Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for CheckedConn {
    fn drop(&mut self) {
        if self.in_transaction.load(Ordering::Relaxed) {
            // Best effort: the connection returns to the pool either way.
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

/// Bind `Value` through rusqlite; booleans are stored as the integers 0 / 1
/// and JSON as its serialized `TEXT` form (SQLite has no JSON type — the
/// `json1` functions read that text). Dates, datetimes and decimals bind
/// their explicit text forms; the conversions are spelled out here rather
/// than pulled from rusqlite's own chrono feature.
impl rusqlite::ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, ValueRef};
        Ok(match self {
            Value::Json(json) => ToSqlOutput::Owned(rusqlite::types::Value::Text(json.to_string())),
            Value::Null => ToSqlOutput::Borrowed(ValueRef::Null),
            Value::Bool(b) => ToSqlOutput::Borrowed(ValueRef::Integer(i64::from(*b))),
            Value::Int(i) => ToSqlOutput::Borrowed(ValueRef::Integer(*i)),
            Value::Float(f) => ToSqlOutput::Borrowed(ValueRef::Real(*f)),
            Value::Text(s) => ToSqlOutput::Borrowed(ValueRef::Text(s.as_bytes())),
            Value::Bytes(b) => ToSqlOutput::Borrowed(ValueRef::Blob(b)),
            #[cfg(feature = "chrono")]
            Value::Date(d) => {
                ToSqlOutput::Owned(rusqlite::types::Value::Text(d.format("%Y-%m-%d").to_string()))
            }
            #[cfg(feature = "chrono")]
            Value::DateTime(dt) => ToSqlOutput::Owned(rusqlite::types::Value::Text(
                dt.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
            )),
            #[cfg(feature = "chrono")]
            Value::DateTimeUtc(dt) => ToSqlOutput::Owned(rusqlite::types::Value::Text(format!(
                "{}Z",
                dt.naive_utc().format("%Y-%m-%dT%H:%M:%S%.f")
            ))),
            #[cfg(feature = "rust_decimal")]
            Value::Decimal(d) => ToSqlOutput::Owned(rusqlite::types::Value::Text(d.to_string())),
        })
    }
}

fn value(v: rusqlite::types::ValueRef<'_>) -> serde_json::Value {
    use rusqlite::types::ValueRef;
    use serde_json::Value;
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => Value::from(f),
        ValueRef::Text(t) => Value::from(String::from_utf8_lossy(t).into_owned()),
        // Blobs become arrays of byte values; swap in base64 if callers need it.
        ValueRef::Blob(b) => Value::from(b.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dropped_transaction_is_rolled_back() {
        let pool = Pool::connect(":memory:", 1).unwrap();
        pool.execute("CREATE TABLE t (x INTEGER)", &[]).await.unwrap();
        {
            let conn = pool.get().unwrap();
            conn.begin().unwrap();
            conn.execute("INSERT INTO t (x) VALUES (1)", &[]).unwrap();
            drop(conn); // mid-transaction: Drop issues ROLLBACK
        }
        let rows = pool.query("SELECT COUNT(*) AS count FROM t", &[]).await.unwrap();
        assert_eq!(rows[0]["count"].as_i64(), Some(0));
    }

    #[tokio::test]
    async fn committed_transaction_survives_drop() {
        let pool = Pool::connect(":memory:", 1).unwrap();
        pool.execute("CREATE TABLE t (x INTEGER)", &[]).await.unwrap();
        {
            let conn = pool.get().unwrap();
            conn.begin().unwrap();
            conn.execute("INSERT INTO t (x) VALUES (1)", &[]).unwrap();
            conn.commit().unwrap();
        }
        let rows = pool.query("SELECT COUNT(*) AS count FROM t", &[]).await.unwrap();
        assert_eq!(rows[0]["count"].as_i64(), Some(1));
    }

    #[test]
    fn status_reports_open_connections() {
        let pool = Pool::connect(":memory:", 4).unwrap();
        let _conn = pool.get().unwrap();
        assert!(pool.status().connections >= 1);
    }
}
