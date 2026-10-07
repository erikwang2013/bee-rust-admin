// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §43/§45/§55: an m2m + JSON model must compile against the real crate —
//! `m2m()` resolves the target's table and primary-key column at runtime, the
//! target ident keeps its attribute spelling, and the `serde_json` spellings
//! map to `SqlType::Json`.
#![allow(dead_code)]

use bee_orm::Model;

#[derive(Model)]
struct Tag {
    id: i64,
    name: String,
}

#[derive(Model)]
struct Category {
    id: i64,
}

#[derive(Model)]
#[bee(m2m(Tag))]
#[bee(m2m(crate::Category, table = "user_category", local = "u_id", foreign = "c_id"))]
struct User {
    id: i64,
    meta: serde_json::Value,
    extra: Option<serde_json::Value>,
}

fn main() {
    let defs = User::m2m();
    assert_eq!(defs.len(), 2);
    assert_eq!(defs[0].table, "user_tag");
    assert_eq!(defs[0].local_column, "user_id");
    assert_eq!(defs[0].foreign_column, "tag_id");
    assert_eq!(defs[0].target_ident, "Tag");
    assert_eq!((defs[0].target_table)(), "tags");
    assert_eq!((defs[0].target_columns)()[0].name, "id");
    assert_eq!(defs[1].table, "user_category");
    assert_eq!(defs[1].local_column, "u_id");
    assert_eq!(defs[1].foreign_column, "c_id");
    // §55: a fully qualified target still carries its last segment verbatim.
    assert_eq!(defs[1].target_ident, "Category");
    assert_eq!((defs[1].target_table)(), "categorys");

    let columns = User::columns();
    assert_eq!(columns[1].name, "meta");
    assert_eq!(columns[1].sql, bee_orm::model::SqlType::Json);
    assert!(!columns[1].nullable);
    assert_eq!(columns[2].sql, bee_orm::model::SqlType::Json);
    assert!(columns[2].nullable);
}
