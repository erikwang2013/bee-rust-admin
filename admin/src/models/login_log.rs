// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "login_log")]
pub struct LoginLog {
    #[bee(pk)]
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub id: i64,
    /// 0 = 未登录（凭据无效/用户不存在）
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub admin_id: i64,
    #[bee(sql_type = "VARCHAR(64)")]
    pub username: String,
    #[bee(sql_type = "VARCHAR(45)")]
    pub ip: String,
    #[bee(sql_type = "VARCHAR(255)")]
    pub user_agent: String,
    /// 1 成功 / 0 失败
    pub status: i8,
    #[bee(sql_type = "VARCHAR(255)")]
    pub msg: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
}
