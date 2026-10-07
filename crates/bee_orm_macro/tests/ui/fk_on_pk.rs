// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §37.2: the primary key cannot also be a foreign key.
#![allow(dead_code)]

#[derive(bee_orm::Model)]
struct Foo {
    #[bee(pk, fk = User)]
    id: i64,
}

fn main() {}
