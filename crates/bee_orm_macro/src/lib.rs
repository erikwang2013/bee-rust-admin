// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! `#[derive(Model)]` for `bee_orm::Model`.
//!
//! Struct level: `#[bee(table = "name")]` overrides the table name (default is
//! the struct name lowercased plus `s`); `#[bee(hooks(before_insert, …))]`
//! forwards each listed lifecycle hook to a same-named inherent async method
//! the user writes (`async fn hook(&self) -> bee_orm::Result<()>`);
//! `#[bee(m2m(Target))]`, repeatable, declares a many-to-many join table
//! towards `Target`. Its defaults are the lowercased idents — table
//! `{local}_{target}`, columns `{local}_id` / `{target}_id` — overridden by
//! `table = "…"`, `local = "…"`, `foreign = "…"`; a target whose two column
//! defaults collide (the model itself, `Self`) needs the explicit columns.
//! `#[bee(crate = "…")]` replaces the `bee_orm` prefix of every emitted path,
//! for models that reach the ORM crate through a re-export.
//! Field level, combinable in one attribute (`#[bee(pk, auto)]`):
//!
//! - `column = "name"` — column override, must match `[A-Za-z_][A-Za-z0-9_]*`
//! - `pk` — primary key; falls back to the field named `id` when unmarked
//! - `auto` — database-assigned, skipped by `insert_values`
//! - `ignore` — not a table column; excluded from every read/write and built
//!   with `Default::default()` in `from_row`
//! - `auto_now_add` / `auto_now` — unix-seconds timestamp columns injected by
//!   the trait (insert only / insert and update); excluded from both value
//!   lists. The field type must decode an integer (`i64` / `Option<i64>`)
//! - `soft_delete` — the soft-delete flag column, stored as a `bool`; stays an
//!   ordinary writable column, and rows whose flag is NULL stay invisible to
//!   default queries
//! - `sql_type = "..."` — raw SQL type for `columns()`, bypassing the type
//!   spelling table; required for spellings without a mapping (`u64`, custom
//!   types) and for a non-integer `auto` primary key
//! - `fk = Target` — foreign key: `columns()` records the target model's table
//!   and primary-key column, and the target must derive `Model`

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod expand;
mod parse;
mod types;

use expand::expand;

/// Derive `bee_orm::Model` for a struct.
///
/// Every knob is an argument of `#[bee(…)]`, at struct level:
///
/// | Key | Meaning |
/// |---|---|
/// | `table = "name"` | Table name. Default: struct name lowercased plus `s`. |
/// | `crate = "path"` | Path to the ORM crate, replacing the default `bee_orm` — for models that reach the crate through a re-export (`my_app::bee_orm`). Must be a string literal holding a path. |
/// | `hooks(a, b, …)` | Forward each listed lifecycle event to a same-named inherent async method (`async fn event(&self) -> orm::Result<()>`); repeatable, duplicates are dropped. |
/// | `m2m(Target, table = "…", local = "…", foreign = "…")` | Many-to-many join table towards `Target`; repeatable, the three options optional (defaults are lowercased-ident conventions: table `{local}_{target}`, columns `{local}_id` / `{target}_id`). |
///
/// … and at field level, combinable in one attribute (`#[bee(pk, auto)]`):
///
/// | Key | Meaning |
/// |---|---|
/// | `column = "name"` | Column override, must match `[A-Za-z_][A-Za-z0-9_]*`. |
/// | `pk` | Primary key. Without it the field named `id` is the key; two marked fields are an error. |
/// | `auto` | Database-assigned: skipped by `insert_values`. Primary key only, and the mapped type must be an integer. |
/// | `ignore` | Not a column: excluded from every read, write and `columns()`, and built with `Default::default()` in `from_row`. |
/// | `auto_now_add` | Unix-seconds timestamp column injected on insert; the field type must decode an integer (`i64` / `Option<i64>`). |
/// | `auto_now` | Same, refreshed on insert and update. |
/// | `soft_delete` | Soft-delete flag column, stored as a `bool`: rows whose flag is NULL stay invisible to default queries. At most one per model, never on the primary key. |
/// | `sql_type = "…"` | Raw SQL type for `columns()`, bypassing the type spelling table; required for spellings without a mapping and for a non-integer `auto` primary key. |
/// | `fk = Target` | Foreign key: `columns()` records `Target`'s table and primary-key column, and `Target` must derive `Model`. |
#[proc_macro_derive(Model, attributes(bee))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(input) {
        Ok(expanded) => expanded.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[cfg(test)]
mod tests;
