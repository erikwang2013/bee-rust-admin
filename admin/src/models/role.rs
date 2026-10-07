// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "role")]
pub struct Role {
    #[bee(pk, auto)]
    pub id: i64,
    #[bee(sql_type = "VARCHAR(64)")]
    pub name: String,
    #[bee(sql_type = "VARCHAR(64)")]
    pub code: String,
    pub sort: i32,
    /// 1 全部 / 2 本部门及以下 / 3 本部门 / 4 仅本人 / 5 自定义
    pub data_scope: i8,
    pub status: i8,
    #[bee(sql_type = "VARCHAR(255)")]
    pub remark: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
