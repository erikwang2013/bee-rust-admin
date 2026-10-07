// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "menu")]
pub struct Menu {
    #[bee(pk, auto)]
    pub id: i64,
    pub parent_id: i64,
    #[bee(sql_type = "VARCHAR(64)")]
    pub name: String,
    /// M 目录 / C 菜单 / F 按钮（列名用 menu_type，JSON 对外叫 type）
    #[serde(rename = "type")]
    #[bee(sql_type = "VARCHAR(255)")]
    pub menu_type: String,
    #[bee(sql_type = "VARCHAR(128)")]
    pub perm: String,
    #[bee(sql_type = "VARCHAR(128)")]
    pub path: String,
    #[bee(sql_type = "VARCHAR(128)")]
    pub component: String,
    #[bee(sql_type = "VARCHAR(64)")]
    pub icon: String,
    pub sort: i32,
    pub visible: i8,
    pub status: i8,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
