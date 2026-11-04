// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! In-crate unit tests: a recording [`Db`] mock, shared model fixtures, and
//! SQL-shape checks per area — [`Model`](crate::Model) operations
//! ([`model_ops`]), [`QuerySet`] ([`queryset`]), [`migrate`](crate::migrate)
//! and [`rel`](crate::rel). Behavior against real backends lives in
//! `tests/**`.

mod datetime_types;
mod fixtures;
mod m2m;
mod migrate;
mod model_ops;
mod queryset;
mod rel;
mod shard;

pub(crate) use fixtures::*;

// Shared by the test modules through their `use super::*`; the fixtures
// themselves live in `fixtures`.
use crate::{ColumnDef, Db, Dialect, Model, OrmError, Result, Row, SqlType, Value};
