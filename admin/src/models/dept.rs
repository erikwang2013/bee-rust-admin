// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "dept")]
pub struct Dept {
    #[bee(pk, auto)]
    pub id: i64,
    pub parent_id: i64,
    #[bee(sql_type = "VARCHAR(64)")]
    pub name: String,
    pub sort: i32,
    #[bee(sql_type = "VARCHAR(64)")]
    pub leader: String,
    #[bee(sql_type = "VARCHAR(20)")]
    pub phone: String,
    pub status: i8,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
