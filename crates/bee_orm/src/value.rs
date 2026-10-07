// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! The [`Value`] enum — a driver-independent bound parameter — plus the
//! [`FromValue`] / [`decode`] helpers that turn a JSON [`Row`] back into a
//! typed value.

use crate::{OrmError, Result, Row};

/// A typed SQL parameter, independent of any backend driver.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    /// A JSON document, bound to the backend's native representation: sqlite
    /// stores its serialized `TEXT` form, postgres binds `json` / `jsonb`,
    /// mysql sends the serialized bytes (a `JSON` column accepts them).
    /// Reading works in the other direction through
    /// [`decode`] / [`FromValue`] for a `serde_json::Value` field.
    Json(serde_json::Value),
    /// A calendar date without a time zone (pg `date`, mysql `date`); sqlite
    /// stores its ISO-8601 `TEXT`.
    #[cfg(feature = "chrono")]
    Date(chrono::NaiveDate),
    /// A timezone-less timestamp (pg `timestamp`, mysql `datetime`); sqlite
    /// stores its ISO-8601 `TEXT`.
    #[cfg(feature = "chrono")]
    DateTime(chrono::NaiveDateTime),
    /// An instant, always held in UTC (pg `timestamptz`, mysql `timestamp`
    /// under a UTC session — mysql has no timezone-aware column type).
    #[cfg(feature = "chrono")]
    DateTimeUtc(chrono::DateTime<chrono::Utc>),
    /// An arbitrary-precision decimal (pg `numeric`, mysql `decimal`); sqlite
    /// stores its text form — a `DECIMAL` declaration would get NUMERIC
    /// affinity and silently turn `"1.50"` into `REAL` 1.5.
    #[cfg(feature = "rust_decimal")]
    Decimal(rust_decimal::Decimal),
}

macro_rules! value_from_int {
    ($($ty:ty),* $(,)?) => { $(
        impl From<$ty> for Value {
            fn from(value: $ty) -> Self { Value::Int(value as i64) }
        }
    )* };
}
value_from_int!(i8, i16, i32, i64, u8, u16, u32); // u64/usize excluded: not lossless

impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Value::Float(value as f64)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::Float(value)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::Text(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::Text(value.to_owned())
    }
}

impl From<&String> for Value {
    fn from(value: &String) -> Self {
        Value::Text(value.clone())
    }
}

impl From<Vec<u8>> for Value {
    fn from(value: Vec<u8>) -> Self {
        Value::Bytes(value)
    }
}

impl From<serde_json::Value> for Value {
    fn from(value: serde_json::Value) -> Self {
        Value::Json(value)
    }
}

impl From<&[u8]> for Value {
    fn from(value: &[u8]) -> Self {
        Value::Bytes(value.to_vec())
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveDate> for Value {
    fn from(value: chrono::NaiveDate) -> Self {
        Value::Date(value)
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveDateTime> for Value {
    fn from(value: chrono::NaiveDateTime) -> Self {
        Value::DateTime(value)
    }
}

/// Only `DateTime<Utc>` maps: local / fixed-offset datetimes have no
/// timezone-neutral column to bind to.
#[cfg(feature = "chrono")]
impl From<chrono::DateTime<chrono::Utc>> for Value {
    fn from(value: chrono::DateTime<chrono::Utc>) -> Self {
        Value::DateTimeUtc(value)
    }
}

#[cfg(feature = "rust_decimal")]
impl From<rust_decimal::Decimal> for Value {
    fn from(value: rust_decimal::Decimal) -> Self {
        Value::Decimal(value)
    }
}

/// `None` binds as [`Value::Null`].
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        match value {
            Some(inner) => inner.into(),
            None => Value::Null,
        }
    }
}

/// Decode seam for one column type; the blanket impl covers every
/// `serde::de::DeserializeOwned` type, so models need no serde derives.
pub trait FromValue: Sized {
    fn from_value(value: serde_json::Value) -> Result<Self>;
}

impl<T: serde::de::DeserializeOwned> FromValue for T {
    fn from_value(value: serde_json::Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| OrmError::QueryError(e.to_string()))
    }
}

/// Decode `column` from `row`; a missing column decodes as JSON `null`.
///
/// SQLite stores booleans as the integers 0/1 and JSON has no integer↔boolean
/// coercion, so a failed decode is retried once with that single normalisation
/// (0/1 number ↔ bool). Every other mismatch reports the original error.
///
/// A `serde_json::Value` target (and `Option<serde_json::Value>`) reads a JSON
/// column in its serialized form: sqlite TEXT and mysql bytes both arrive as a
/// string cell, which is parsed — invalid JSON is reported as a decode error,
/// never silently kept as a string. Postgres `json` / `jsonb` columns deliver
/// parsed JSON (strings and `null` re-serialized), which passes through
/// untouched.
///
/// `Option<serde_json::Value>` keeps SQL `NULL` and the JSON document `null`
/// apart: SQL `NULL` is `None`, a stored `null` document is `Some(Json::Null)`
/// — the distinction is decided on the raw cell, before parsing would erase
/// it.
pub fn decode<T: FromValue + 'static>(row: &Row, column: &str) -> Result<T> {
    use std::any::TypeId;

    use serde_json::Value as Json;
    let cell = row.get(column).cloned().unwrap_or(Json::Null);
    // The `Option<serde_json::Value>` case cannot go through `from_value`:
    // a bare `Json::Null` deserializes to `None` whether it came from SQL
    // NULL or from the serialized `null` document.
    if TypeId::of::<T>() == TypeId::of::<Option<Json>>() {
        let decoded = optional_json_cell(cell, column)?;
        let boxed: Box<dyn std::any::Any> = Box::new(decoded);
        let typed = boxed.downcast::<T>().expect("TypeId checked to be Option<serde_json::Value>");
        return Ok(*typed);
    }
    let value = json_cell::<T>(cell, column)?;
    let decoded = T::from_value(value.clone()).or_else(|first| match bool_normalised(&value) {
        Some(normalised) => T::from_value(normalised).map_err(|_| first),
        None => Err(first),
    });
    decoded.map_err(|e| OrmError::QueryError(format!("column `{column}`: {e}")))
}

/// The cell a `serde_json::Value` target sees: a string cell is a serialized
/// document and is parsed; `null` and already-parsed JSON cells pass through —
/// as does every cell for any other target type. A `serde_json::Value` field
/// over a plain TEXT column is read as JSON by this rule too: valid JSON
/// parses, anything else is an error.
fn json_cell<T: 'static>(value: serde_json::Value, column: &str) -> Result<serde_json::Value> {
    use std::any::TypeId;

    use serde_json::Value as Json;
    match (TypeId::of::<T>() == TypeId::of::<Json>(), value) {
        (true, Json::String(text)) => parse_json(&text, column),
        (_, other) => Ok(other),
    }
}

/// The cell an `Option<serde_json::Value>` target sees. Only SQL `NULL` is
/// `None`; a string cell is the serialized document and is parsed, so a stored
/// `null` stays `Some(Json::Null)`.
fn optional_json_cell(value: serde_json::Value, column: &str) -> Result<Option<serde_json::Value>> {
    use serde_json::Value as Json;
    match value {
        Json::Null => Ok(None),
        Json::String(text) => parse_json(&text, column).map(Some),
        other => Ok(Some(other)),
    }
}

fn parse_json(text: &str, column: &str) -> Result<serde_json::Value> {
    serde_json::from_str(text)
        .map_err(|e| OrmError::QueryError(format!("column `{column}`: invalid JSON: {e}")))
}

/// `0`/`1` as a boolean, or a boolean as `0`/`1`; `None` for anything else.
fn bool_normalised(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(0) => Some(serde_json::Value::Bool(false)),
            Some(1) => Some(serde_json::Value::Bool(true)),
            _ => None,
        },
        serde_json::Value::Bool(b) => Some(serde_json::Value::from(i64::from(*b))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row_with_active(active: i64) -> Row {
        let mut row = Row::new();
        row.insert("active".into(), json!(active));
        row
    }

    #[test]
    fn from_conversions() {
        assert_eq!(Value::from(1u8), Value::Int(1));
        assert_eq!(Value::from(None::<i32>), Value::Null);
        assert_eq!(Value::from(Some(3i32)), Value::Int(3));
        assert_eq!(Value::from(json!({ "a": 1 })), Value::Json(json!({ "a": 1 })));
    }

    #[test]
    fn json_text_cells_parse_and_invalid_json_errors() {
        let mut row = Row::new();
        row.insert("payload".into(), json!("{\"a\":1}"));
        assert_eq!(decode::<serde_json::Value>(&row, "payload").unwrap(), json!({ "a": 1 }));
        assert_eq!(
            decode::<Option<serde_json::Value>>(&row, "payload").unwrap(),
            Some(json!({ "a": 1 }))
        );

        row.insert("payload".into(), json!("[1,2]"));
        assert_eq!(decode::<serde_json::Value>(&row, "payload").unwrap(), json!([1, 2]));

        // A serialized `null` document is `Some(null)`, not SQL NULL.
        row.insert("payload".into(), json!("null"));
        assert_eq!(decode::<serde_json::Value>(&row, "payload").unwrap(), serde_json::Value::Null);
        assert_eq!(
            decode::<Option<serde_json::Value>>(&row, "payload").unwrap(),
            Some(serde_json::Value::Null)
        );

        // A serialized string scalar: the quotes are the JSON, not the text.
        row.insert("payload".into(), json!("\"hi\""));
        assert_eq!(decode::<serde_json::Value>(&row, "payload").unwrap(), json!("hi"));
        assert_eq!(
            decode::<Option<serde_json::Value>>(&row, "payload").unwrap(),
            Some(json!("hi"))
        );

        // Every json-target read of the same row agrees on invalid text.
        row.insert("payload".into(), json!("{oops"));
        let err = decode::<Option<serde_json::Value>>(&row, "payload").err().unwrap();
        assert!(err.to_string().contains("invalid JSON"), "{err}");
        let err = decode::<serde_json::Value>(&row, "payload").err().unwrap();
        assert!(err.to_string().contains("invalid JSON"), "{err}");
    }

    #[test]
    fn json_cells_that_are_already_parsed_pass_through() {
        let mut row = Row::new();
        row.insert("payload".into(), json!({ "b": 2 }));
        assert_eq!(decode::<serde_json::Value>(&row, "payload").unwrap(), json!({ "b": 2 }));

        row.insert("payload".into(), serde_json::Value::Null);
        assert_eq!(decode::<Option<serde_json::Value>>(&row, "payload").unwrap(), None);
        assert_eq!(decode::<serde_json::Value>(&row, "payload").unwrap(), serde_json::Value::Null);
    }

    #[test]
    fn decode_bool_from_sqlite_integer() {
        let row = row_with_active(1);
        assert!(decode::<bool>(&row, "active").unwrap());
        assert_eq!(decode::<Option<bool>>(&row, "active").unwrap(), Some(true));
    }

    #[test]
    fn decode_rejects_non_boolean_integers() {
        let row = row_with_active(5);
        assert!(decode::<bool>(&row, "active").is_err());
    }

    #[test]
    fn missing_column_decodes_as_null() {
        let row = Row::new();
        assert_eq!(decode::<Option<i32>>(&row, "age").unwrap(), None);
        assert!(decode::<i32>(&row, "age").is_err());
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn chrono_values_convert_and_round_trip() {
        use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
        let date = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        let naive = date.and_hms_micro_opt(12, 34, 56, 123456).unwrap();
        let utc = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);

        assert_eq!(Value::from(date), Value::Date(date));
        assert_eq!(Value::from(naive), Value::DateTime(naive));
        assert_eq!(Value::from(utc), Value::DateTimeUtc(utc));

        // Reads go through the serde blanket: the cell a backend hands over is
        // the value's serde JSON form, and it decodes back to the same value.
        let mut row = Row::new();
        row.insert("v".into(), serde_json::to_value(date).unwrap());
        assert_eq!(decode::<NaiveDate>(&row, "v").unwrap(), date);
        row.insert("v".into(), serde_json::to_value(naive).unwrap());
        assert_eq!(decode::<NaiveDateTime>(&row, "v").unwrap(), naive);
        row.insert("v".into(), serde_json::to_value(utc).unwrap());
        assert_eq!(decode::<DateTime<Utc>>(&row, "v").unwrap(), utc);
    }

    #[cfg(feature = "rust_decimal")]
    #[test]
    fn decimal_values_convert_and_round_trip() {
        use rust_decimal::Decimal;
        let value: Decimal = "1.50".parse().unwrap();
        assert_eq!(Value::from(value), Value::Decimal(value));

        let mut row = Row::new();
        row.insert("v".into(), serde_json::to_value(value).unwrap());
        // The serde form is the exact string, so the scale survives the trip.
        assert_eq!(row["v"], json!("1.50"));
        assert_eq!(decode::<Decimal>(&row, "v").unwrap(), value);
    }
}
