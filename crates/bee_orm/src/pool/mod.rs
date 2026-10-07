// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Connection pools, one submodule per backend, each enabled by a cargo
//! feature (`sqlite`, `postgres`, `mysql`).
//!
//! Every backend exposes the same shape:
//!
//! - `Pool::connect(dsn, max_size)` — build the pool (connections are opened
//!   lazily).
//! - `Pool::get()` — check out one connection and hold it. Use this for
//!   transactions: statements sent through the `Pool` itself may each land on
//!   a different connection.
//! - `Pool::query(sql, params)` / `Pool::execute(sql, params)` — check out,
//!   run, check in.
//!
//! Parameters are the `&[Value]` that
//! [`QuerySet::params`](crate::QuerySet::params) returns and are always bound
//! through the driver's prepared-statement interface, never interpolated. On
//! a soft-delete model the `QuerySet` execution methods add one implicit
//! `flag = ?` parameter ahead of these and bind it themselves; a manual
//! `to_sql()` + `params()` pairing does not.
//!
//! # Transactions
//!
//! Transactions must run on a connection held from `Pool::get()`: statements
//! sent through the `Pool` itself may each land on a different connection.
//! `CheckedConn::begin` / `commit` / `rollback` issue `BEGIN` / `COMMIT` /
//! `ROLLBACK`. Always end a transaction with `commit()` or `rollback()` before
//! the `CheckedConn` goes out of scope — dropping one mid-transaction is
//! cleaned up on a best-effort basis (see each backend module: sqlite and
//! postgres roll back on drop, mysql relies on the pool's check-out reset).

/// One result row: column name → JSON value.
pub type Row = serde_json::Map<String, serde_json::Value>;

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
use crate::OrmError;

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
pub(crate) fn conn_err(e: impl std::fmt::Display) -> OrmError {
    OrmError::ConnectionError(e.to_string())
}

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
pub(crate) fn query_err(e: impl std::fmt::Display) -> OrmError {
    OrmError::QueryError(e.to_string())
}

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "mysql")]
pub mod mysql;
