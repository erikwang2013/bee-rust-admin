// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §28.3/§28.4: a `hooks(...)` model must compile against the real crate —
//! the generated forwarders have to resolve to the inherent methods (reaching
//! the trait default instead would recurse at runtime).
#![allow(dead_code)]

use bee_orm::Model;

#[derive(Model)]
#[bee(table = "orders", hooks(before_insert, after_delete))]
struct Order {
    id: i64,
    note: String,
}

impl Order {
    async fn before_insert(&self) -> bee_orm::Result<()> {
        Ok(())
    }

    async fn after_delete(&self) -> bee_orm::Result<()> {
        Ok(())
    }
}

fn main() {}
