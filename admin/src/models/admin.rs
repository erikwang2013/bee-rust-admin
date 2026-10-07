// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "admin")]
pub struct Admin {
    #[bee(pk, auto)]
    pub id: i64,
    #[bee(sql_type = "VARCHAR(64)")]
    pub username: String,
    #[serde(skip_serializing, default)]
    #[bee(sql_type = "VARCHAR(255)")]
    pub password: String,
    #[bee(sql_type = "VARCHAR(64)")]
    pub nickname: String,
    #[bee(sql_type = "VARCHAR(128)")]
    pub email: String,
    #[bee(sql_type = "VARCHAR(20)")]
    pub phone: String,
    pub sex: i8,
    #[bee(sql_type = "VARCHAR(255)")]
    pub avatar: String,
    pub dept_id: i64,
    pub status: i8,
    pub is_super: i8,
    pub token_version: i32,
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    pub last_login_at: Option<NaiveDateTime>,
    #[bee(sql_type = "VARCHAR(45)")]
    pub last_login_ip: String,
    #[bee(sql_type = "VARCHAR(255)")]
    pub remark: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
