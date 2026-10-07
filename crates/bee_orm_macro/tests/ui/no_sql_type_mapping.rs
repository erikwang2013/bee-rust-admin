// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §34/§37.2: a spelling outside the type table (deliberately unmapped
//! `u64`) needs `#[bee(sql_type = "...")]`.
#![allow(dead_code)]

#[derive(bee_orm::Model)]
struct Counter {
    id: i64,
    hits: u64,
}

fn main() {}
