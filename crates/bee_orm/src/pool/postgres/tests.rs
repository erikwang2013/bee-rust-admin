// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Unit tests for the postgres pool: `Value` encoding, pool-error mapping and
//! the TLS config builder. Moved out of `postgres.rs` verbatim when the file
//! hit the 500-line ceiling.

use super::*;
use tokio_postgres::types::{IsNull, Type, private::BytesMut};

fn encode(value: &Value, ty: &Type) -> std::result::Result<(IsNull, Vec<u8>), String> {
    let mut buf = BytesMut::new();
    match <Value as ToSql>::to_sql(value, ty, &mut buf) {
        Ok(is_null) => Ok((is_null, buf.to_vec())),
        Err(e) => Err(e.to_string()),
    }
}

#[test]
fn int4_narrows_to_the_column_type() {
    let (is_null, bytes) = encode(&Value::Int(5), &Type::INT4).unwrap();
    assert!(matches!(is_null, IsNull::No));
    assert_eq!(bytes, 5i32.to_be_bytes().to_vec());
}

#[test]
fn int4_overflow_is_an_error() {
    let over = i64::from(i32::MAX) + 1;
    assert!(encode(&Value::Int(over), &Type::INT4).is_err());
}

#[test]
fn int2_overflow_is_an_error() {
    let over = i64::from(i16::MAX) + 1;
    assert!(encode(&Value::Int(over), &Type::INT2).is_err());
}

#[test]
fn float4_overflow_is_an_error() {
    assert!(encode(&Value::Float(f64::MAX), &Type::FLOAT4).is_err());
}

#[test]
fn null_is_null_yes_and_writes_nothing() {
    let (is_null, bytes) = encode(&Value::Null, &Type::INT4).unwrap();
    assert!(matches!(is_null, IsNull::Yes));
    assert!(bytes.is_empty());
}

#[test]
fn text_on_int4_is_an_error() {
    assert!(encode(&Value::Text("x".into()), &Type::INT4).is_err());
}

#[test]
fn json_binds_to_json_and_jsonb_columns() {
    for ty in [Type::JSON, Type::JSONB] {
        let (is_null, bytes) = encode(&Value::Json(serde_json::json!({ "a": 1 })), &ty).unwrap();
        assert!(matches!(is_null, IsNull::No));
        let payload = if ty == Type::JSONB { &bytes[1..] } else { &bytes[..] };
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(payload).unwrap(),
            serde_json::json!({ "a": 1 })
        );
    }
}

#[test]
fn json_on_text_is_an_error() {
    assert!(encode(&Value::Json(serde_json::json!(1)), &Type::TEXT).is_err());
}

#[test]
fn json_string_cells_are_reserialized_for_the_text_form() {
    // `jsonb` hands a JSON string back as a Rust string; the cell carries the
    // serialized form so it matches sqlite TEXT / mysql bytes.
    assert_eq!(json_pass(Ok(Some(Json::String("hi".into())))), Json::String("\"hi\"".into()));
    assert_eq!(json_pass(Ok(Some(Json::from(1)))), Json::from(1));
    assert_eq!(json_pass(Ok(None)), Json::Null);
    // A stored `null` document is not SQL NULL: it keeps the text form so
    // `Option<serde_json::Value>` can tell `Some(null)` from `None`.
    assert_eq!(json_pass(Ok(Some(Json::Null))), Json::String("null".into()));
}

#[test]
fn wait_timeout_maps_to_pool_exhausted() {
    let err = pool_err(deadpool_postgres::PoolError::Timeout(deadpool_postgres::TimeoutType::Wait));
    assert!(matches!(&err, OrmError::ConnectionError(msg) if msg.contains("pool exhausted")));
}

#[test]
fn other_pool_errors_keep_the_driver_text() {
    let err =
        pool_err(deadpool_postgres::PoolError::Timeout(deadpool_postgres::TimeoutType::Create));
    assert!(matches!(&err, OrmError::ConnectionError(msg) if !msg.contains("pool exhausted")));
}

#[test]
fn fresh_pool_reports_no_open_connections() {
    // Works without a runtime: connections open lazily and no timeout is
    // polled until `get()` waits.
    let pool = Pool::connect("postgres://localhost/app", 4).unwrap();
    let status = pool.status();
    assert_eq!(status.max_size, 4);
    assert_eq!(status.size, 0);
}

#[cfg(feature = "postgres-tls")]
#[test]
fn rustls_config_builds_with_bundled_roots() {
    rustls_client_config().unwrap();
    assert!(!webpki_roots::TLS_SERVER_ROOTS.is_empty());
}
