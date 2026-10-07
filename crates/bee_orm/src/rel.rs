// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Relation reads over foreign-key metadata: `belongs_to` and `has_many`,
//! including the batched [`children_for`] anti-N+1 path.
//!
//! Read-only: no join builder, no many-to-many, no eager attachment. All
//! reads go through [`QuerySet`], so each model's own soft-delete filter
//! applies — soft-deleted children are excluded, [`belongs_to`] on a
//! soft-deleted parent is `None`; the query-builder variants
//! ([`children_query`] / [`children_query_via`]) are the way to
//! `with_deleted()`. Deleting a parent never touches children rows.
//!
//! The foreign-key column is [`references`](crate::ColumnDef::references)
//! metadata (`#[bee(fk = Target)]`); without it the `children*` helpers error
//! with both model names and the attribute to add. Writing a child is an
//! ordinary `insert` / `update` with the fk column set by hand.

use std::collections::{HashMap, HashSet};

use serde_json::Value as Json;

use crate::{Db, MAX_BIND_PARAMS, Model, OrmError, QuerySet, Result, Value};

/// Declaration order: the first column of `C` that references `P`.
pub fn fk_column_to<C: Model, P: Model>() -> Option<&'static str> {
    C::columns().iter().find_map(|column| match &column.references {
        Some(reference) if (reference.table)() == P::table_name() => Some(column.name),
        _ => None,
    })
}

/// A query for the children of `parent`, via the discovered fk column.
///
/// The child's soft-delete filter applies; chain `with_deleted()` on the
/// result to escape it.
pub fn children_query<C: Model, P: Model>(parent: &P) -> Result<QuerySet<C>> {
    let fk_column = fk_column_to::<C, P>().ok_or_else(missing_fk::<C, P>)?;
    children_query_via::<C, P>(parent, fk_column)
}

/// [`children_query`] with an explicit fk column — for a second foreign key
/// to the same parent.
pub fn children_query_via<C: Model, P: Model>(parent: &P, fk_column: &str) -> Result<QuerySet<C>> {
    QuerySet::<C>::new(C::table_name()).filter_eq(fk_column, parent.pk_value())
}

/// The children of `parent`, via the discovered fk column.
pub async fn children<C: Model, P: Model, D: Db + ?Sized>(db: &D, parent: &P) -> Result<Vec<C>> {
    let fk_column = fk_column_to::<C, P>().ok_or_else(missing_fk::<C, P>)?;
    children_via::<C, P, D>(db, parent, fk_column).await
}

/// [`children`] with an explicit fk column.
pub async fn children_via<C: Model, P: Model, D: Db + ?Sized>(
    db: &D,
    parent: &P,
    fk_column: &str,
) -> Result<Vec<C>> {
    children_query_via::<C, P>(parent, fk_column)?.all(db).await
}

/// The children of every `parent` in one `IN` query per 999-key chunk —
/// **index-aligned with the input**, so the same parent twice gets its group
/// twice. No parents, no query.
pub async fn children_for<C: Model, P: Model, D: Db + ?Sized>(
    db: &D,
    parents: &[P],
) -> Result<Vec<Vec<C>>> {
    let fk_column = fk_column_to::<C, P>().ok_or_else(missing_fk::<C, P>)?;
    children_for_via::<C, P, D>(db, parents, fk_column).await
}

/// [`children_for`] with an explicit fk column.
pub async fn children_for_via<C: Model, P: Model, D: Db + ?Sized>(
    db: &D,
    parents: &[P],
    fk_column: &str,
) -> Result<Vec<Vec<C>>> {
    if parents.is_empty() {
        return Ok(Vec::new());
    }
    // Dedupe parent keys: `IN` with a repeated key buys nothing.
    let mut seen: HashSet<String> = HashSet::new();
    let mut values: Vec<Value> = Vec::new();
    for parent in parents {
        let value = parent.pk_value();
        if seen.insert(grouping_key(&json_of(&value))) {
            values.push(value);
        }
    }
    let mut rows = Vec::new();
    for chunk in values.chunks(MAX_BIND_PARAMS) {
        let query = QuerySet::<C>::new(C::table_name()).filter_in(fk_column, chunk)?;
        rows.extend(query.rows(db).await?);
    }
    // Group row indices by the fk value, then decode per parent below —
    // decoding once per parent occurrence keeps duplicates aligned without
    // requiring `C: Clone`.
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        if let Some(value) = row.get(fk_column) {
            groups.entry(grouping_key(value)).or_default().push(index);
        }
    }
    let mut result = Vec::with_capacity(parents.len());
    for parent in parents {
        let key = grouping_key(&json_of(&parent.pk_value()));
        let Some(indices) = groups.get(&key) else {
            result.push(Vec::new());
            continue;
        };
        let mut group = Vec::with_capacity(indices.len());
        for &index in indices {
            group.push(C::from_row(&rows[index])?);
        }
        result.push(group);
    }
    Ok(result)
}

/// The row `fk` points at: `P`'s pk `= fk`, first match.
///
/// A `NULL` fk matches nothing (`= NULL` SQL semantics) → `None`; the
/// parent's soft-delete filter applies, so a soft-deleted parent is `None`.
pub async fn belongs_to<P: Model, D: Db + ?Sized>(
    db: &D,
    fk: impl Into<Value>,
) -> Result<Option<P>> {
    QuerySet::<P>::new(P::table_name()).filter_eq(P::pk_column(), fk)?.one(db).await
}

fn missing_fk<C: Model, P: Model>() -> OrmError {
    OrmError::QueryError(format!(
        "rel: no column of `{}` references `{}`; add #[bee(fk = {})] to the foreign-key field",
        C::table_name(),
        P::table_name(),
        P::table_name()
    ))
}

/// The grouping key of an fk / pk JSON value: its canonical JSON text, with
/// booleans normalised to `1` / `0` — sqlite and mysql read a bool column
/// back as an integer while the parent side holds [`Value::Bool`], and both
/// spellings must land in the same group.
pub(crate) fn grouping_key(value: &Json) -> String {
    match value {
        Json::Bool(flag) => i64::from(*flag).to_string(),
        other => other.to_string(),
    }
}

/// [`Value`] as JSON. Not a `From` impl on purpose: `Value` stays
/// driver-independent, and the grouping callers are the only ones needing it.
pub(crate) fn json_of(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(flag) => Json::Bool(*flag),
        Value::Int(int) => Json::from(*int),
        Value::Float(float) => Json::from(*float),
        Value::Text(text) => Json::from(text.clone()),
        Value::Bytes(bytes) => Json::from(bytes.clone()),
        Value::Json(json) => json.clone(),
        // The same serde form the read path hands over, so a bound value and
        // a decoded cell group on one key.
        #[cfg(feature = "chrono")]
        Value::Date(date) => serde_json::to_value(date).unwrap_or(Json::Null),
        #[cfg(feature = "chrono")]
        Value::DateTime(datetime) => serde_json::to_value(datetime).unwrap_or(Json::Null),
        #[cfg(feature = "chrono")]
        Value::DateTimeUtc(datetime) => serde_json::to_value(datetime).unwrap_or(Json::Null),
        #[cfg(feature = "rust_decimal")]
        Value::Decimal(decimal) => serde_json::to_value(decimal).unwrap_or(Json::Null),
    }
}
