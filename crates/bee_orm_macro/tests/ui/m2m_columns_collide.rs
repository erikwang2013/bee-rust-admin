// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §43: a same-ident target makes both column defaults resolve equal —
//! explicit `local` / `foreign` overrides are required.
#![allow(dead_code)]

#[derive(bee_orm::Model)]
#[bee(m2m(Tag))]
struct Tag {
    id: i64,
}

fn main() {}
