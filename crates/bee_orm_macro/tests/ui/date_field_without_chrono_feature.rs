// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §56: the date spellings map unconditionally, so a model using one
//! while `bee_orm`'s `chrono` feature is off fails at compile time on the
//! missing `From<NaiveDate> for Value` — never a silently wrong column.
//! (`chrono` is a dev-dependency here only so the type resolves; this crate
//! never enables `bee_orm/chrono`.)
#![allow(dead_code)]

use chrono::NaiveDate;

#[derive(bee_orm::Model)]
struct Event {
    id: i64,
    day: NaiveDate,
}

fn main() {}
