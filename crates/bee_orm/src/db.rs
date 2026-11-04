// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! The [`Db`] trait: the execution seam every query builder and model method
//! talks to, so callers can accept `&dyn Db` regardless of backend.
//!
//! Implementations are provided by each `pool` backend. Bind values always go
//! through the driver's prepared-statement interface; the SQL string never
//! contains user data.

use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::{Dialect, Result, Row, Value};

#[async_trait]
pub trait Db: Send + Sync {
    /// Run a query and collect every row.
    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>>;

    /// Run a statement, returning the number of affected rows.
    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64>;

    /// The backend's SQL dialect, which [`migrate`](crate::migrate) needs for
    /// DDL. `None` by default — mock and hand-written impls keep compiling;
    /// the migration functions fail loudly on `None` rather than guessing.
    fn dialect(&self) -> Option<Dialect> {
        None
    }

    /// The physical table behind a logical name; the identity by default.
    ///
    /// A sharded backend overrides this to pick the table's shard. Statements
    /// that address a model table go through here before formatting; the
    /// returned name is spliced into SQL unquoted, so it must match
    /// `[A-Za-z_][A-Za-z0-9_]*`.
    fn resolve_table(&self, logical: &str) -> Result<String> {
        Ok(logical.to_owned())
    }

    /// Insert and, where the backend can, return the stored row.
    ///
    /// The seam behind [`Model::create`](crate::Model::create), which needs
    /// the row as stored to surface database-assigned values (an
    /// auto-increment primary key, column defaults). The default runs the
    /// statement through [`execute`](Db::execute) and returns `Ok(None)` —
    /// mocks and hand-written impls keep compiling, and `create` falls back
    /// to selecting by primary key. `sql` is a plain `INSERT`; `table` is the
    /// resolved physical table name and `pk_column` is the model's static
    /// primary-key column, for backends whose read-back needs them.
    /// `Ok(None)` also covers a backend that ran the statement but cannot
    /// produce the row.
    async fn insert_returning(
        &self,
        _table: &str,
        _pk_column: &str,
        sql: &str,
        params: &[Value],
    ) -> Result<Option<Row>> {
        self.execute(sql, params).await?;
        Ok(None)
    }

    /// [`insert_returning`](Db::insert_returning) with the model's hint of
    /// whether the primary key is assigned by the database.
    ///
    /// `auto_increment` is [`ColumnDef::auto_increment`](crate::ColumnDef) for
    /// the pk column, read from [`Model::columns`](crate::Model::columns) — a
    /// static metadata hint, not a schema query. `None` means the model carries
    /// no metadata (a hand-written impl), and the backend should probe as
    /// before. A backend whose read-back depends on it (MySQL's
    /// `LAST_INSERT_ID()` is only meaningful for an `AUTO_INCREMENT` column)
    /// uses it to skip probing the catalog; the default ignores it and forwards
    /// to `insert_returning`.
    ///
    /// # Schema drift
    ///
    /// The hint is trusted: [`Model::columns`](crate::Model::columns) must
    /// match the physical schema. A table whose pk is (or is not)
    /// `AUTO_INCREMENT` while the metadata says otherwise is unsupported; a
    /// backend that skips the catalog probe may then return a stale row or
    /// fall back to the pk select.
    async fn insert_returning_auto(
        &self,
        table: &str,
        pk_column: &str,
        auto_increment: Option<bool>,
        sql: &str,
        params: &[Value],
    ) -> Result<Option<Row>> {
        let _ = auto_increment;
        self.insert_returning(table, pk_column, sql, params).await
    }
}

/// A [`Db`] wrapper that reports `query` / `execute` calls and insert
/// read-backs ([`insert_returning`](Db::insert_returning) /
/// [`insert_returning_auto`](Db::insert_returning_auto)) taking at least
/// `slow` to `on_slow(sql, elapsed)`. Forwarding preserves the backend's
/// read-back; the wrapper only measures it.
///
/// # Panics
///
/// The callback runs inline after the statement completes; it must not panic.
/// A panic unwinds the caller and discards the statement's (successful) result.
pub struct TimedDb<D> {
    inner: D,
    slow: Duration,
    on_slow: SlowHandler,
}

type SlowHandler = Box<dyn Fn(&str, Duration) + Send + Sync>;

impl<D> TimedDb<D> {
    pub fn new(
        inner: D,
        slow: Duration,
        on_slow: impl Fn(&str, Duration) + Send + Sync + 'static,
    ) -> Self {
        Self { inner, slow, on_slow: Box::new(on_slow) }
    }

    pub fn inner(&self) -> &D {
        &self.inner
    }

    fn observe(&self, sql: &str, elapsed: Duration) {
        if elapsed >= self.slow {
            (self.on_slow)(sql, elapsed);
        }
    }
}

#[async_trait]
impl<D: Db> Db for TimedDb<D> {
    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let start = Instant::now();
        let result = self.inner.query(sql, params).await;
        self.observe(sql, start.elapsed());
        result
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        let start = Instant::now();
        let result = self.inner.execute(sql, params).await;
        self.observe(sql, start.elapsed());
        result
    }

    fn dialect(&self) -> Option<Dialect> {
        self.inner.dialect()
    }

    fn resolve_table(&self, logical: &str) -> Result<String> {
        self.inner.resolve_table(logical)
    }

    async fn insert_returning(
        &self,
        table: &str,
        pk_column: &str,
        sql: &str,
        params: &[Value],
    ) -> Result<Option<Row>> {
        let start = Instant::now();
        let result = self.inner.insert_returning(table, pk_column, sql, params).await;
        self.observe(sql, start.elapsed());
        result
    }

    async fn insert_returning_auto(
        &self,
        table: &str,
        pk_column: &str,
        auto_increment: Option<bool>,
        sql: &str,
        params: &[Value],
    ) -> Result<Option<Row>> {
        let start = Instant::now();
        let result =
            self.inner.insert_returning_auto(table, pk_column, auto_increment, sql, params).await;
        self.observe(sql, start.elapsed());
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct MockDb {
        executed: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Db for MockDb {
        async fn query(&self, sql: &str, _params: &[Value]) -> Result<Vec<Row>> {
            if sql == "slow" {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok(Vec::new())
        }

        async fn execute(&self, sql: &str, _params: &[Value]) -> Result<u64> {
            self.executed.lock().unwrap().push(sql.to_owned());
            if sql == "slow" {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok(1)
        }

        fn dialect(&self) -> Option<Dialect> {
            Some(Dialect::Sqlite)
        }

        async fn insert_returning_auto(
            &self,
            _table: &str,
            _pk_column: &str,
            auto_increment: Option<bool>,
            sql: &str,
            _params: &[Value],
        ) -> Result<Option<Row>> {
            if sql == "slow" {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let mut row = Row::new();
            row.insert("auto_increment".into(), serde_json::json!(auto_increment));
            Ok(Some(row))
        }
    }

    #[tokio::test]
    async fn slow_query_and_execute_report_elapsed() {
        let seen: Arc<Mutex<Vec<(String, Duration)>>> = Arc::default();
        let db = TimedDb::new(MockDb::default(), Duration::from_millis(5), {
            let seen = Arc::clone(&seen);
            move |sql: &str, elapsed| seen.lock().unwrap().push((sql.to_owned(), elapsed))
        });
        db.query("slow", &[]).await.unwrap();
        db.execute("slow", &[]).await.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(
            seen.iter().all(|(sql, elapsed)| sql == "slow" && *elapsed >= Duration::from_millis(5))
        );
    }

    #[tokio::test]
    async fn fast_statements_stay_quiet() {
        let fired = Arc::new(AtomicUsize::new(0));
        let db = TimedDb::new(MockDb::default(), Duration::from_secs(60), {
            let fired = Arc::clone(&fired);
            move |_, _| {
                fired.fetch_add(1, Ordering::Relaxed);
            }
        });
        db.query("fast", &[]).await.unwrap();
        db.execute("fast", &[]).await.unwrap();
        assert_eq!(fired.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn slow_insert_read_backs_report_elapsed() {
        let seen: Arc<Mutex<Vec<(String, Duration)>>> = Arc::default();
        let db = TimedDb::new(MockDb::default(), Duration::from_millis(5), {
            let seen = Arc::clone(&seen);
            move |sql: &str, elapsed| seen.lock().unwrap().push((sql.to_owned(), elapsed))
        });
        db.insert_returning("t", "id", "slow", &[]).await.unwrap();
        db.insert_returning_auto("t", "id", Some(true), "slow", &[]).await.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(
            seen.iter().all(|(sql, elapsed)| sql == "slow" && *elapsed >= Duration::from_millis(5))
        );
    }

    #[tokio::test]
    async fn insert_returning_auto_is_forwarded_not_executed() {
        let db = TimedDb::new(MockDb::default(), Duration::from_secs(60), |_, _| {});
        assert_eq!(db.dialect(), Some(Dialect::Sqlite));
        let row = db
            .insert_returning_auto("t", "id", Some(true), "INSERT INTO t DEFAULT VALUES", &[])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row["auto_increment"], serde_json::json!(true));
        assert!(db.inner().executed.lock().unwrap().is_empty());
    }
}
