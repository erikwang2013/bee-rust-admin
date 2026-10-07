// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §18.3: the duplicate-`table` error combines with other accumulated
//! errors instead of short-circuiting them.
#![allow(dead_code)]

#[derive(bee_orm::Model)]
#[bee(table = "a", table = "b", nope)]
struct Foo {
    id: i64,
}

fn main() {}
