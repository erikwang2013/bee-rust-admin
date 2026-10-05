// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "login_log", pk = "id")]
pub struct LoginLog {
    #[bee(auto)]
    pub id: u64,
    pub admin_id: u64,
    pub username: String,
    pub ip: String,
    pub user_agent: String,
    /// 1 成功 / 0 失败
    pub status: i8,
    pub msg: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
}
