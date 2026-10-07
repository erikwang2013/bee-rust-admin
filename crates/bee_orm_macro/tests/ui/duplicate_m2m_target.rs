// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §43: one `m2m(...)` per target — a repeat is a compile error.
#![allow(dead_code)]

#[derive(bee_orm::Model)]
#[bee(m2m(Tag))]
#[bee(m2m(Tag))]
struct User {
    id: i64,
}

fn main() {}
