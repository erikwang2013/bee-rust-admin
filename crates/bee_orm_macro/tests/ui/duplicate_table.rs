// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §18.3: a second `#[bee(table = …)]` is a compile error, not last-wins.
#![allow(dead_code)]

#[derive(bee_orm::Model)]
#[bee(table = "a", table = "b")]
struct Foo {
    id: i64,
}

fn main() {}
