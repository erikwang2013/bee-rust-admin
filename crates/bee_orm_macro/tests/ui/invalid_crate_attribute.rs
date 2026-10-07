// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §61.2: every bad `#[bee(crate = "…")]` shape fails loudly — a silently
//! ignored path would mis-point every emitted path of the model at once.
use bee_orm::Model;

#[derive(Model)]
#[bee(crate = 42)]
struct NotALiteral {
    id: i64,
}

#[derive(Model)]
#[bee(crate = "")]
struct Empty {
    id: i64,
}

#[derive(Model)]
#[bee(crate = "bee_orm Model")]
struct NotAPath {
    id: i64,
}

#[derive(Model)]
#[bee(crate = "bee_orm", crate = "bee_orm")]
struct Duplicate {
    id: i64,
}

fn main() {}
