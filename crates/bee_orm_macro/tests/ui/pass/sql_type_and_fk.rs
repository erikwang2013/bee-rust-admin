// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §37.4: `sql_type` and `fk` in one attribute must compile against the
//! real crate — the raw type string wins the type, and `columns()` records a
//! `Reference` resolving to the target model's table and primary-key column.
#![allow(dead_code)]

use bee_orm::Model;

#[derive(Model)]
struct User {
    id: i64,
    name: String,
}

#[derive(Model)]
#[bee(table = "posts")]
struct Post {
    #[bee(pk, auto)]
    id: i32,
    #[bee(sql_type = "VARCHAR(64)", fk = User)]
    author: String,
    created_at: i64,
}

fn main() {
    let columns = Post::columns();
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].name, "id");
    assert_eq!(columns[0].sql, bee_orm::model::SqlType::Int);
    assert!(columns[0].auto_increment);
    assert_eq!(columns[1].name, "author");
    assert_eq!(columns[1].sql, bee_orm::model::SqlType::Raw("VARCHAR(64)"));
    assert!(!columns[1].nullable);
    let reference = columns[1].references.expect("`author` declares a foreign key");
    assert_eq!((reference.table)(), "users");
    assert_eq!((reference.pk_column)(), "id");
    assert_eq!(columns[2].name, "created_at");
    assert_eq!(columns[2].sql, bee_orm::model::SqlType::BigInt);
}
