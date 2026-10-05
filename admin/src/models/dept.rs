// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "dept", pk = "id")]
pub struct Dept {
    #[bee(auto)]
    pub id: u64,
    pub parent_id: u64,
    pub name: String,
    pub sort: i32,
    pub leader: String,
    pub phone: String,
    pub status: i8,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
