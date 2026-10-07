// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Many-to-many relations over the join tables declared by
//! `#[bee(m2m(Target))]` (the [`M2mDef`] metadata of [`Model::m2m`]).
//!
//! The join table is not a `Model`, so reads are two steps — raw SQL for the
//! foreign ids, then a [`QuerySet`] `filter_in` for the rows — never a JOIN.
//! Step 2 means the target's soft-delete filter applies: a soft-deleted
//! target disappears from [`related`] / [`related_for`]. [`related_ids`] is
//! the escape hatch when the caller wants its own ordering, limits or
//! `with_deleted()`.
//!
//! Join rows are never cleaned up: deleting either side leaves them behind,
//! and an id with no matching row is simply dropped from the result.
//! [`attach`] / [`detach`] write the join table only — the models' hooks and
//! timestamp columns do not run.
//!
//! ```no_run
//! # async fn demo(pool: &impl bee_orm::Db) -> Result<(), bee_orm::OrmError> {
//! # use bee_orm::Model;
//! # #[derive(Model, Clone)] struct User { id: i64 }
//! # #[derive(Model, Clone)] struct Tag { id: i64 }
//! let user = User { id: 1 };
//! let tag = Tag { id: 7 };
//! bee_orm::m2m::attach(pool, &user, &tag).await?;
//! let tags: Vec<Tag> = bee_orm::m2m::related(pool, &user).await?;
//! # Ok(())
//! # }
//! ```

use std::collections::{HashMap, HashSet};

use serde_json::Value as Json;

use crate::rel::{grouping_key, json_of};
use crate::{Db, M2mDef, MAX_BIND_PARAMS, Model, OrmError, QuerySet, Result, Value};

/// The join table of `L` targeting `R`, or `None` when `L` declares no m2m
/// relation to `R`.
pub fn m2m_def<L: Model, R: Model>() -> Option<&'static M2mDef> {
    L::m2m().iter().find(|def| (def.target_table)() == R::table_name())
}

/// The target ids related to `local`, in join-row (database) order — an `IN`
/// list for a caller-built query, e.g.
/// `R::query().filter_in(R::pk_column(), &ids)`. Duplicates cannot occur
/// (the join table's primary key is the pair).
pub async fn related_ids<L: Model, R: Model, D: Db + ?Sized>(
    db: &D,
    local: &L,
) -> Result<Vec<Value>> {
    let def = def_or_err::<L, R>()?;
    let sql =
        format!("SELECT {} FROM {} WHERE {} = ?", def.foreign_column, def.table, def.local_column);
    let rows = db.query(&sql, &[local.pk_value()]).await?;
    Ok(rows.iter().filter_map(|row| row.get(def.foreign_column)).map(as_value).collect())
}

/// The targets related to `local`. No ordering guarantee (join-row order is
/// database-defined); use [`related_ids`] for a custom query.
pub async fn related<L: Model, R: Model, D: Db + ?Sized>(db: &D, local: &L) -> Result<Vec<R>> {
    let ids = related_ids::<L, R, D>(db, local).await?;
    let mut result = Vec::new();
    for chunk in ids.chunks(MAX_BIND_PARAMS) {
        let query = QuerySet::<R>::new(R::table_name()).filter_in(R::pk_column(), chunk)?;
        result.extend(query.all(db).await?);
    }
    Ok(result)
}

/// The targets of every `local` in one chunked pair of queries — **index
/// aligned with the input**, so the same local twice gets its group twice.
/// No locals, no query.
///
/// An id with no matching target row (deleted, or dangling on mysql) is
/// dropped; that local gets a short or empty group.
pub async fn related_for<L: Model, R: Model, D: Db + ?Sized>(
    db: &D,
    locals: &[L],
) -> Result<Vec<Vec<R>>> {
    let def = def_or_err::<L, R>()?;
    if locals.is_empty() {
        return Ok(Vec::new());
    }
    // Step 1: the (local, foreign) pairs, one `IN` per 999-key chunk. Local
    // keys are deduped first; `IN` with a repeated key buys nothing.
    let mut seen: HashSet<String> = HashSet::new();
    let mut keys: Vec<Value> = Vec::new();
    for local in locals {
        let value = local.pk_value();
        if seen.insert(grouping_key(&json_of(&value))) {
            keys.push(value);
        }
    }
    let mut pairs: HashMap<String, Vec<Json>> = HashMap::new();
    for chunk in keys.chunks(MAX_BIND_PARAMS) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let sql = format!(
            "SELECT {} AS local_key, {} AS foreign_key FROM {} WHERE {} IN ({placeholders})",
            def.local_column, def.foreign_column, def.table, def.local_column
        );
        for row in db.query(&sql, chunk).await? {
            if let (Some(local_key), Some(foreign_key)) =
                (row.get("local_key"), row.get("foreign_key"))
            {
                pairs.entry(grouping_key(local_key)).or_default().push(foreign_key.clone());
            }
        }
    }
    // Step 2: every distinct foreign id in one chunked `filter_in`, then
    // group the loaded rows by pk value. Decoding once per row below keeps
    // duplicate locals aligned without requiring `R: Clone`.
    let mut foreign_ids: Vec<Value> = Vec::new();
    let mut seen_foreign: HashSet<String> = HashSet::new();
    for pair in pairs.values().flatten() {
        if seen_foreign.insert(grouping_key(pair)) {
            foreign_ids.push(as_value(pair));
        }
    }
    let mut rows = Vec::new();
    for chunk in foreign_ids.chunks(MAX_BIND_PARAMS) {
        let query = QuerySet::<R>::new(R::table_name()).filter_in(R::pk_column(), chunk)?;
        rows.extend(query.rows(db).await?);
    }
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        if let Some(value) = row.get(R::pk_column()) {
            groups.entry(grouping_key(value)).or_default().push(index);
        }
    }
    let mut result = Vec::with_capacity(locals.len());
    for local in locals {
        let key = grouping_key(&json_of(&local.pk_value()));
        let mut group = Vec::new();
        for foreign_key in pairs.get(&key).into_iter().flatten() {
            for &index in groups.get(&grouping_key(foreign_key)).into_iter().flatten() {
                group.push(R::from_row(&rows[index])?);
            }
        }
        result.push(group);
    }
    Ok(result)
}

/// Relation `local` → `foreign`: one plain `INSERT` of both primary-key
/// values. Re-attaching an existing pair is the join table's primary-key
/// error, not a silent no-op.
pub async fn attach<L: Model, R: Model, D: Db + ?Sized>(
    db: &D,
    local: &L,
    foreign: &R,
) -> Result<u64> {
    let def = def_or_err::<L, R>()?;
    let sql = format!(
        "INSERT INTO {} ({}, {}) VALUES (?, ?)",
        def.table, def.local_column, def.foreign_column
    );
    db.execute(&sql, &[local.pk_value(), foreign.pk_value()]).await
}

/// Remove the relation: `DELETE` of the pair, idempotent — a second call
/// affects 0 rows.
pub async fn detach<L: Model, R: Model, D: Db + ?Sized>(
    db: &D,
    local: &L,
    foreign: &R,
) -> Result<u64> {
    let def = def_or_err::<L, R>()?;
    let sql = format!(
        "DELETE FROM {} WHERE {} = ? AND {} = ?",
        def.table, def.local_column, def.foreign_column
    );
    db.execute(&sql, &[local.pk_value(), foreign.pk_value()]).await
}

/// The def or the loud error naming both models and the attribute to add.
///
/// The attribute hint must spell the *target ident*, not its table name (a
/// renamed table is not a type and would not compile). The forward direction
/// being absent leaves no way to read `R`'s ident, so the hint takes it from
/// the reverse declaration `R` → `L` when that exists, and falls back to a
/// `<Target>` placeholder otherwise.
fn def_or_err<L: Model, R: Model>() -> Result<&'static M2mDef> {
    m2m_def::<L, R>().ok_or_else(|| {
        let hint = match m2m_def::<R, L>() {
            Some(reverse) => format!(
                "`{}` declares the reverse `#[bee(m2m({}))]` — call from `{}`, or declare \
                 `#[bee(m2m(<Target>))]` on `{}`",
                R::table_name(),
                reverse.target_ident,
                R::table_name(),
                L::table_name()
            ),
            None => format!(
                "declare `#[bee(m2m(<Target>))]` on `{}` (or the reverse on `{}`)",
                L::table_name(),
                R::table_name()
            ),
        };
        OrmError::QueryError(format!(
            "m2m: no relation from `{}` to `{}`; {hint}",
            L::table_name(),
            R::table_name()
        ))
    })
}

/// A row cell (or join-cell) back as a bound-parameter [`Value`]: numbers
/// narrow to `Int` when they are integers, JSON text stays text, and an
/// array/object cell (a JSON primary key is not a thing, but a non-JSON
/// backend cell shape could be) stays [`Value::Json`].
fn as_value(json: &Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(number) => match number.as_i64() {
            Some(int) => Value::Int(int),
            None => Value::Float(number.as_f64().unwrap_or_default()),
        },
        Json::String(text) => Value::Text(text.clone()),
        other => Value::Json(other.clone()),
    }
}
