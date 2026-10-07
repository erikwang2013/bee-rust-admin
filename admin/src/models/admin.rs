// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "admin")]
pub struct Admin {
    /// 主键由代码给（雪花发号，见 `AppState::next_id`）：没有 `#[bee(auto)]`，
    /// insert 时把 id 一起写进去；对外 JSON 里是 hashids 短串。
    #[bee(pk)]
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub id: i64,
    #[bee(sql_type = "VARCHAR(64)")]
    pub username: String,
    #[serde(skip_serializing, default)]
    #[bee(sql_type = "VARCHAR(255)")]
    pub password: String,
    #[bee(sql_type = "VARCHAR(64)")]
    pub nickname: String,
    /// 落库存密文（见 `crate::crypto`）：255 是**密文**宽度（明文 + 29 字节再 base64，
    /// ≈4/3 倍），业务上限 128 字符明文写不满。旧库的列窄（128/20），启动时的
    /// 列宽迁移会改过来（`seed::migrate`）—— 上游 migrate 只加列不改类型。
    #[bee(sql_type = "VARCHAR(255)")]
    pub email: String,
    /// 同上：落库是密文，列宽按密文取（20 位明文 → 约 68 字符密文）
    #[bee(sql_type = "VARCHAR(255)")]
    pub phone: String,
    pub sex: i8,
    #[bee(sql_type = "VARCHAR(255)")]
    pub avatar: String,
    /// 0 = 无部门（哨兵值，编码成短串后仍要能解回 0）
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
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
