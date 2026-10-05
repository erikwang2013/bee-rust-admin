// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "admin", pk = "id")]
pub struct Admin {
    #[bee(auto)]
    pub id: u64,
    #[bee(unique)]
    #[bee(len = 64)]
    pub username: String,
    #[serde(skip_serializing, default)]
    pub password: String,
    #[bee(len = 64)]
    pub nickname: String,
    #[bee(len = 128)]
    pub email: String,
    #[bee(len = 20)]
    pub phone: String,
    pub sex: i8,
    pub avatar: String,
    #[bee(index)]
    pub dept_id: u64,
    #[bee(index)]
    pub status: i8,
    pub is_super: i8,
    pub token_version: i32,
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    pub last_login_at: Option<NaiveDateTime>,
    #[bee(len = 45)]
    pub last_login_ip: String,
    pub remark: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
