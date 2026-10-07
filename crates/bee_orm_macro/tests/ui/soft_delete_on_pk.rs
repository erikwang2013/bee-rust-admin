// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §28.2: the soft-delete flag cannot be the primary key.
#![allow(dead_code)]

#[derive(bee_orm::Model)]
struct Foo {
    #[bee(pk, soft_delete)]
    id: i64,
}

fn main() {}
