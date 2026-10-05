// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "menu", pk = "id")]
pub struct Menu {
    #[bee(auto)]
    pub id: u64,
    pub parent_id: u64,
    pub name: String,
    /// M 目录 / C 菜单 / F 按钮（列名用 menu_type，JSON 对外叫 type）
    #[serde(rename = "type")]
    pub menu_type: String,
    pub perm: String,
    pub path: String,
    pub component: String,
    pub icon: String,
    pub sort: i32,
    pub visible: i8,
    pub status: i8,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
