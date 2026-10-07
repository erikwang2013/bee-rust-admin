// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! bee_orm — a small async ORM: typed bound parameters ([`Value`]), connection
//! pools per backend, a fluent query builder ([`QuerySet`]) and a model trait
//! ([`Model`]) with default `INSERT` / `UPDATE` / `DELETE`. [`migrate`] builds
//! non-destructive DDL from the model metadata, [`rel`] reads foreign-key
//! relations and [`m2m`] many-to-many ones over join tables.

pub mod db;
pub mod m2m;
pub mod migrate;
pub mod model;
pub mod pool;
pub mod queryset;
pub mod rel;
pub mod value;

#[cfg(test)]
mod tests;

pub use bee_orm_macro::Model;
pub use db::Db;
pub use migrate::MigrateOptions;
pub use model::{ColumnDef, DefaultValue, Dialect, M2mDef, Model, Reference, SqlType};
pub use pool::Row;
pub use queryset::QuerySet;
pub use value::{FromValue, Value, decode};

/// The `rustls` this crate links against (feature `postgres-tls`):
/// `Pool::connect_tls_with` takes a `rustls::ClientConfig`, and building one
/// from this re-export guarantees the caller names the same version.
///
/// ```
/// // Nameable from outside the crate — the point of the re-export. Pass one
/// // to `Pool::connect_tls_with`.
/// let config: Option<bee_orm::rustls::ClientConfig> = None;
/// assert!(config.is_none());
/// ```
#[cfg(feature = "postgres-tls")]
pub use rustls;

/// The conservative per-statement bound-parameter ceiling, shared by
/// [`Model::insert_many`] and [`rel::children_for`] chunking.
///
/// ponytail: 999 is sqlite's classic bound; pg/mysql allow 65535 — the
/// conservative floor for every backend, one constant, uniform SQL.
pub(crate) const MAX_BIND_PARAMS: usize = 999;

/// Implementation details re-exported for `bee_orm_macro`'s generated code
/// (the macro annotates hook-forwarding impls with `#[bee_orm::__private::async_trait]`).
/// Not public API — no stability guarantees.
#[doc(hidden)]
pub mod __private {
    pub use async_trait::async_trait;
}

/// Result type used throughout this crate.
pub type Result<T> = std::result::Result<T, OrmError>;

#[derive(Debug, thiserror::Error)]
pub enum OrmError {
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("query error: {0}")]
    QueryError(String),
    #[error("invalid field name: {0}")]
    InvalidField(String),
    #[error("not found")]
    NotFound,
}
