// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 字典类型。`code` 是字典项的关联键（不建外键，与现有风格一致），建成后不可改。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "dict_type")]
pub struct DictType {
    #[bee(pk)]
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub id: i64,
    #[bee(sql_type = "VARCHAR(64)")]
    pub name: String,
    #[bee(sql_type = "VARCHAR(64)")]
    pub code: String,
    /// 1 启用 / 0 停用
    pub status: i8,
    #[bee(sql_type = "VARCHAR(255)")]
    pub remark: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}

/// 字典项。`(type_code, value)` 的复合唯一键 syncdb 表达不了（它只建单列唯一），
/// 由 `seed::migrate` 的裸 DDL 建；其余列与索引仍走 syncdb。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "dict_item")]
pub struct DictItem {
    #[bee(pk)]
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub id: i64,
    #[bee(sql_type = "VARCHAR(64)")]
    pub type_code: String,
    #[bee(sql_type = "VARCHAR(64)")]
    pub label: String,
    #[bee(sql_type = "VARCHAR(64)")]
    pub value: String,
    pub sort: i32,
    /// 1 启用 / 0 停用
    pub status: i8,
    #[bee(sql_type = "VARCHAR(255)")]
    pub remark: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
