// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 系统配置（扩展工程项 1）：键值对 + 类型 / 分组 / 公开标记。
///
/// 列名用 `config_key` / `group_name`：`key` / `group` 是 MySQL 保留字，
/// 而 bee_orm 拼 SQL **不加引号标识符** —— 列名撞保留字是语法错误，
/// 不是转义问题。Rust / JSON 字段仍是 `key` / `group`（`#[bee(column = …)]` 映射）。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "sys_config")]
pub struct SysConfig {
    #[bee(pk)]
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub id: i64,
    /// 配置键：小写字母/数字/下划线，点号分段（`site.copyright`），2~128 位，建成不可改。
    #[bee(column = "config_key", sql_type = "VARCHAR(128)")]
    pub key: String,
    /// 配置值。一律字符串落库，`bool` 类型在公开读时转 JSON 布尔（见 `api::config`）。
    #[bee(sql_type = "TEXT")]
    pub value: String,
    /// `string` | `text` | `bool`（决定前端表单控件与公开读的取值形状）。
    #[bee(sql_type = "VARCHAR(16)")]
    pub value_type: String,
    /// 分组：管理页按它分卡片；空串 = 未分组。
    #[bee(column = "group_name", sql_type = "VARCHAR(64)")]
    pub group: String,
    /// 1 公开（`/configs/public` 免登录可读 / 0 私有）。
    pub is_public: i8,
    #[bee(sql_type = "VARCHAR(255)")]
    pub remark: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
