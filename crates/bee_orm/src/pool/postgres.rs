// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! PostgreSQL pool (deadpool-postgres).
//!
//! `?` placeholders in the SQL (what [`QuerySet`](crate::QuerySet) emits) are
//! rewritten to PostgreSQL's `$1 … $n` before execution.
//!
//! # Prepared-statement cache
//!
//! `CheckedConn::query` / `execute` run through `prepare_cached`: prepared
//! statements are cached per connection, keyed by (SQL text, parameter types).
//! The cache is **unbounded** — it grows with the number of distinct
//! statements the connection ever sees, and entries are never evicted
//! automatically. If DDL invalidates cached plans, clear it with
//! `pool.manager().statement_caches.clear()`.
//!
//! # Transactions
//!
//! Transactions must run on a connection held from `Pool::get()`: statements
//! sent through the `Pool` itself may each land on a different connection.
//! `CheckedConn::begin` / `commit` / `rollback` issue `BEGIN` / `COMMIT` /
//! `ROLLBACK`. A `CheckedConn` dropped mid-transaction spawns a `ROLLBACK` on
//! the current tokio runtime before the connection returns to the pool;
//! without a runtime (or during shutdown) it cannot, and the connection
//! returns with the transaction open. Only transactions started with
//! `begin()` are tracked — a raw `execute("BEGIN", &[])` is invisible to the
//! drop hook. Always end a transaction with `commit()` or `rollback()`.
//!
//! # TLS
//!
//! `Pool::connect_tls` needs the `postgres-tls` feature and uses the bundled
//! Mozilla roots. `Pool::connect_tls_with` takes a caller-supplied
//! `rustls::ClientConfig` (build it from the `bee_orm::rustls` re-export) for
//! private CAs, client certificates or a custom verifier.
//!
//! ```no_run
//! # async fn demo() -> Result<(), bee_orm::OrmError> {
//! use bee_orm::pool::postgres::Pool;
//!
//! let pool = Pool::connect("postgres://user:pass@localhost:5432/app", 16)?;
//! let rows = pool
//!     .query("SELECT name FROM users WHERE age > ?", &[bee_orm::Value::from(18)])
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use deadpool_postgres::{Manager, ManagerConfig, Pool as DeadpoolPool, Timeouts};
use serde_json::Value as Json;
use tokio_postgres::types::ToSql;

use super::{Row, conn_err, query_err};
use crate::{Db, Dialect, OrmError, Result, Value};

/// A cloneable handle to a deadpool PostgreSQL pool.
#[derive(Clone)]
pub struct Pool {
    inner: DeadpoolPool,
}

impl Pool {
    /// `dsn` is anything `tokio_postgres::Config` accepts, e.g.
    /// `"postgres://user:pass@host:5432/db"` or `"host=… user=… dbname=…"`.
    pub fn connect(dsn: &str, max_size: u32) -> Result<Self> {
        let pg: tokio_postgres::Config = dsn.parse().map_err(conn_err)?;
        let manager = Manager::from_config(pg, tokio_postgres::NoTls, ManagerConfig::default());
        let inner = DeadpoolPool::builder(manager)
            .max_size(max_size.max(1) as usize)
            // ponytail: fixed timeouts — turn into a PoolConfig parameter when a
            // caller needs other values.
            .timeouts(Timeouts {
                wait: Some(Duration::from_secs(30)),
                create: Some(Duration::from_secs(10)),
                recycle: None,
            })
            // deadpool only enforces the timeouts above when a runtime is
            // named; without this line every `connect` fails at build time.
            .runtime(deadpool_postgres::Runtime::Tokio1)
            .build()
            .map_err(conn_err)?;
        Ok(Self { inner })
    }

    /// Like [`Pool::connect`], with TLS driven by the DSN's `sslmode`
    /// (tokio-postgres default: `prefer`; `sslmode=disable` performs no TLS).
    ///
    /// Verification uses the bundled Mozilla roots (ring provider); for a
    /// caller-supplied [`rustls::ClientConfig`] (private CA, custom verifier)
    /// use [`connect_tls_with`](Pool::connect_tls_with).
    #[cfg(feature = "postgres-tls")]
    pub fn connect_tls(dsn: &str, max_size: u32) -> Result<Self> {
        Self::connect_tls_with(dsn, max_size, rustls_client_config()?)
    }

    /// [`connect_tls`](Pool::connect_tls) with a caller-supplied
    /// [`rustls::ClientConfig`] — the injection point for private CAs and
    /// test verifiers.
    ///
    /// Build the config from the re-exported `bee_orm::rustls` so it is the
    /// exact version this pool links against (`ClientConfig` is `Clone`:
    /// one config can serve several pools).
    #[cfg(feature = "postgres-tls")]
    pub fn connect_tls_with(
        dsn: &str,
        max_size: u32,
        config: rustls::ClientConfig,
    ) -> Result<Self> {
        let pg: tokio_postgres::Config = dsn.parse().map_err(conn_err)?;
        let tls = tokio_postgres_rustls::MakeRustlsConnect::new(config);
        let manager = Manager::from_config(pg, tls, ManagerConfig::default());
        let inner = DeadpoolPool::builder(manager)
            .max_size(max_size.max(1) as usize)
            // ponytail: fixed timeouts — turn into a PoolConfig parameter when a
            // caller needs other values.
            .timeouts(Timeouts {
                wait: Some(Duration::from_secs(30)),
                create: Some(Duration::from_secs(10)),
                recycle: None,
            })
            // deadpool only enforces the timeouts above when a runtime is
            // named; without this line every `connect` fails at build time.
            .runtime(deadpool_postgres::Runtime::Tokio1)
            .build()
            .map_err(conn_err)?;
        Ok(Self { inner })
    }

    /// Check out one connection. Hold it for the length of a transaction
    /// (`begin` … `commit` / `rollback`).
    pub async fn get(&self) -> Result<CheckedConn> {
        let conn = self.inner.get().await.map_err(pool_err)?;
        Ok(CheckedConn { conn: Some(conn), in_transaction: AtomicBool::new(false) })
    }

    pub async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        self.get().await?.query(sql, params).await
    }

    /// Returns the number of affected rows.
    pub async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        self.get().await?.execute(sql, params).await
    }

    /// Pool statistics: `max_size`, `size` (open connections), `available`,
    /// `waiting`.
    pub fn status(&self) -> deadpool_postgres::Status {
        self.inner.status()
    }
}

/// Wait-timeout errors get a distinct message; every other pool error keeps
/// the driver's text.
fn pool_err(e: deadpool_postgres::PoolError) -> OrmError {
    match e {
        deadpool_postgres::PoolError::Timeout(deadpool_postgres::TimeoutType::Wait) => {
            OrmError::ConnectionError(
                "pool exhausted: timed out waiting for a connection (postgres, 30s)".into(),
            )
        }
        e => conn_err(e),
    }
}

#[async_trait]
impl Db for Pool {
    fn dialect(&self) -> Option<Dialect> {
        Some(Dialect::Postgres)
    }

    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        Pool::query(self, sql, params).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        Pool::execute(self, sql, params).await
    }

    /// `INSERT … RETURNING *` — the stored row, read on the one connection
    /// that ran the statement.
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

/// A checked-out PostgreSQL connection, returned to the pool on drop.
pub struct CheckedConn {
    conn: Option<deadpool_postgres::Object>,
    in_transaction: AtomicBool,
}

impl CheckedConn {
    fn client(&self) -> &deadpool_postgres::Object {
        self.conn.as_ref().expect("connection is taken only in Drop")
    }

    pub async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let statement = self.client().prepare_cached(&pg_sql(sql)).await.map_err(query_err)?;
        let rows = self.client().query(&statement, &bind(params)).await.map_err(query_err)?;
        Ok(rows.iter().map(row_to_json).collect())
    }

    pub async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        let statement = self.client().prepare_cached(&pg_sql(sql)).await.map_err(query_err)?;
        self.client().execute(&statement, &bind(params)).await.map_err(query_err)
    }

    /// `BEGIN` on this connection. See the module docs on transactions.
    pub async fn begin(&self) -> Result<()> {
        self.execute("BEGIN", &[]).await?;
        self.in_transaction.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// `COMMIT` the transaction started with [`CheckedConn::begin`].
    pub async fn commit(&self) -> Result<()> {
        self.execute("COMMIT", &[]).await?;
        self.in_transaction.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// `ROLLBACK` the transaction started with [`CheckedConn::begin`].
    pub async fn rollback(&self) -> Result<()> {
        self.execute("ROLLBACK", &[]).await?;
        self.in_transaction.store(false, Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for CheckedConn {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else { return };
        if !self.in_transaction.load(Ordering::Relaxed) {
            return; // dropped here: straight back to the pool
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = conn.execute("ROLLBACK", &[]).await;
                // `conn` dropped here: returned to the pool after the rollback
            });
        }
        // No runtime: best effort — the connection returns with the
        // transaction open. Documented, not an error path.
    }
}

fn bind(params: &[Value]) -> Vec<&(dyn ToSql + Sync)> {
    params.iter().map(|p| p as &(dyn ToSql + Sync)).collect()
}

/// Rewrite `?` placeholders into `$1 … $n`.
///
/// ponytail: blind replace — `QuerySet` only ever emits `?` as placeholders,
/// and raw `filter()` strings are trusted constants. A literal `?` inside a
/// string literal in hand-written SQL would be rewritten too.
fn pg_sql(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut n = 0u32;
    for c in sql.chars() {
        if c == '?' {
            n += 1;
            out.push('$');
            out.push_str(&n.to_string());
        } else {
            out.push(c);
        }
    }
    out
}

fn row_to_json(row: &tokio_postgres::Row) -> Row {
    let mut out = Row::new();
    for (i, col) in row.columns().iter().enumerate() {
        out.insert(col.name().to_string(), value(row, i));
    }
    out
}

/// ponytail: decodes the common column types; array columns and the
/// timestamp-without-time-zone relatives still come back as null. Date/time
/// and numeric decode with the `chrono` / `rust_decimal` features on, and
/// fall back to the `_` arm's null when they are off (unchanged behavior).
fn value(row: &tokio_postgres::Row, i: usize) -> Json {
    match row.columns()[i].type_().name() {
        "bool" => opt::<bool>(row.try_get(i)),
        "int2" => opt::<i16>(row.try_get(i)),
        "int4" => opt::<i32>(row.try_get(i)),
        "int8" => opt::<i64>(row.try_get(i)),
        "float4" => opt::<f32>(row.try_get(i)),
        "float8" => opt::<f64>(row.try_get(i)),
        "json" | "jsonb" => json_pass(row.try_get(i)),
        "bytea" => opt::<Vec<u8>>(row.try_get(i)),
        #[cfg(feature = "chrono")]
        "date" => serde_opt::<chrono::NaiveDate>(row.try_get(i)),
        #[cfg(feature = "chrono")]
        "timestamp" => serde_opt::<chrono::NaiveDateTime>(row.try_get(i)),
        #[cfg(feature = "chrono")]
        "timestamptz" => serde_opt::<chrono::DateTime<chrono::Utc>>(row.try_get(i)),
        #[cfg(feature = "rust_decimal")]
        "numeric" => serde_opt::<rust_decimal::Decimal>(row.try_get(i)),
        // text, varchar, char(n), name, enums-as-text …
        _ => opt::<String>(row.try_get(i)),
    }
}

fn opt<T: Into<Json>>(v: std::result::Result<Option<T>, tokio_postgres::Error>) -> Json {
    v.ok().flatten().map(Into::into).unwrap_or(Json::Null)
}

/// [`opt`] for the serde-typed cells (chrono / rust_decimal): the cell is the
/// value's serde JSON form, which `decode` parses back symmetrically. A
/// driver read failure reads as `Json::Null`, like a failed [`opt`].
#[cfg(any(feature = "chrono", feature = "rust_decimal"))]
fn serde_opt<T: serde::Serialize>(
    v: std::result::Result<Option<T>, tokio_postgres::Error>,
) -> Json {
    match v {
        Ok(Some(value)) => serde_json::to_value(value).unwrap_or(Json::Null),
        _ => Json::Null,
    }
}

/// [`opt`] for `json` / `jsonb`, with one normalisation: a JSON *string* is
/// re-serialized (`"hi"`, quotes included), because sqlite stores the TEXT
/// form and mysql hands bytes — a bare `hi` cell would be indistinguishable
/// from a plain TEXT column, and `decode` parses the serialized form for a
/// `serde_json::Value` field. For the same reason a JSON `null` document is
/// re-serialized as the text `null`: SQL NULL *is* the `Json::Null` cell, and
/// `Option<serde_json::Value>` must keep the two apart. Objects, arrays and
/// the remaining scalars pass through as-is. A `String`-typed field over a
/// `json` column therefore sees the quoted form; declare the field
/// `serde_json::Value` (the `SqlType::Json` spelling).
fn json_pass(v: std::result::Result<Option<Json>, tokio_postgres::Error>) -> Json {
    match v {
        Ok(Some(Json::String(s))) => Json::String(Json::String(s).to_string()),
        Ok(Some(Json::Null)) => Json::String("null".to_string()),
        Ok(Some(other)) => other,
        // SQL NULL — and, as in `opt`, a failed read.
        _ => Json::Null,
    }
}

/// Encode `Value` for the PostgreSQL wire, choosing the binary format from the
/// column's type as reported by the prepared statement.
///
/// Integers are narrowed to the target type and **never truncated**: an
/// out-of-range value is an error.
impl ToSql for Value {
    fn to_sql(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> std::result::Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    {
        use tokio_postgres::types::IsNull;
        match (self, ty.name()) {
            (Value::Null, _) => Ok(IsNull::Yes),
            (Value::Bool(v), "bool") => {
                out.extend_from_slice(&[u8::from(*v)]);
                Ok(IsNull::No)
            }
            (Value::Int(v), "int2") => {
                out.extend_from_slice(
                    &i16::try_from(*v).map_err(|_| range_error(v, "int2"))?.to_be_bytes(),
                );
                Ok(IsNull::No)
            }
            (Value::Int(v), "int4") => {
                out.extend_from_slice(
                    &i32::try_from(*v).map_err(|_| range_error(v, "int4"))?.to_be_bytes(),
                );
                Ok(IsNull::No)
            }
            (Value::Int(v), "int8") => {
                out.extend_from_slice(&v.to_be_bytes());
                Ok(IsNull::No)
            }
            (Value::Float(v), "float4") => {
                out.extend_from_slice(&narrow_f32(*v)?.to_be_bytes());
                Ok(IsNull::No)
            }
            (Value::Float(v), "float8") => {
                out.extend_from_slice(&v.to_be_bytes());
                Ok(IsNull::No)
            }
            (Value::Text(v), "text" | "varchar" | "bpchar" | "name" | "unknown") => {
                out.extend_from_slice(v.as_bytes());
                Ok(IsNull::No)
            }
            (Value::Bytes(v), "bytea") => {
                out.extend_from_slice(v);
                Ok(IsNull::No)
            }
            // The server infers the parameter type at prepare; the driver's
            // serde_json impl emits the text form (jsonb gets its version
            // byte), so `Value::Json` binds to both json and jsonb columns.
            (Value::Json(v), "json" | "jsonb") => v.to_sql(ty, out),
            #[cfg(feature = "chrono")]
            (Value::Date(d), "date") => d.to_sql(ty, out),
            #[cfg(feature = "chrono")]
            (Value::DateTime(dt), "timestamp") => dt.to_sql(ty, out),
            #[cfg(feature = "chrono")]
            (Value::DateTimeUtc(dt), "timestamptz") => dt.to_sql(ty, out),
            #[cfg(feature = "rust_decimal")]
            (Value::Decimal(d), "numeric") => d.to_sql(ty, out),
            (value, target) => Err(mismatch_error(value, target)),
        }
    }

    fn accepts(_ty: &tokio_postgres::types::Type) -> bool {
        true
    }

    /// Equivalent to what `postgres_types::to_sql_checked!()` generates, with
    /// the `accepts` check folded out: this impl accepts every type and reports
    /// a mismatch from `to_sql` instead (messages name the offending column
    /// type).
    fn to_sql_checked(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> std::result::Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    {
        self.to_sql(ty, out)
    }
}

/// Narrow `f64` to `f32`, rejecting conversions that lose the value entirely:
/// overflow to infinity, underflow to zero. In-range rounding is unavoidable
/// and allowed.
fn narrow_f32(value: f64) -> std::result::Result<f32, Box<dyn std::error::Error + Sync + Send>> {
    let narrowed = value as f32;
    let lost = (narrowed.is_infinite() && value.is_finite()) || (narrowed == 0.0 && value != 0.0);
    if lost { Err(range_error(value, "float4")) } else { Ok(narrowed) }
}

fn range_error(
    value: impl std::fmt::Display,
    target: &str,
) -> Box<dyn std::error::Error + Sync + Send> {
    format!("value `{value}` does not fit PostgreSQL {target}").into()
}

/// Names the variant only — never the data — so error messages cannot leak
/// values.
fn mismatch_error(value: &Value, target: &str) -> Box<dyn std::error::Error + Sync + Send> {
    let kind = match value {
        Value::Null => "Null",
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Text(_) => "Text",
        Value::Bytes(_) => "Bytes",
        Value::Json(_) => "Json",
        #[cfg(feature = "chrono")]
        Value::Date(_) => "Date",
        #[cfg(feature = "chrono")]
        Value::DateTime(_) => "DateTime",
        #[cfg(feature = "chrono")]
        Value::DateTimeUtc(_) => "DateTimeUtc",
        #[cfg(feature = "rust_decimal")]
        Value::Decimal(_) => "Decimal",
    };
    format!("cannot bind Value::{kind} as PostgreSQL {target}").into()
}

/// Bundled Mozilla roots (webpki-roots). The crypto provider is named
/// explicitly on purpose: workspace builds enable both rustls providers
/// (aws-lc-rs through other crates, ring here) and `ClientConfig::builder()`
/// panics when the process-level default is ambiguous.
/// `MakeRustlsConnect::with_webpki_roots()` has the same hazard — it
/// documents that it uses the process default.
#[cfg(feature = "postgres-tls")]
fn rustls_client_config() -> Result<rustls::ClientConfig> {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(conn_err)?
        .with_root_certificates(rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        })
        .with_no_client_auth();
    Ok(config)
}

#[cfg(test)]
mod tests;
