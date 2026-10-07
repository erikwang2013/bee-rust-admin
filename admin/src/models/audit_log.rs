// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 操作日志（审计）：中间件对写操作自动落一条。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "audit_log")]
pub struct AuditLog {
    #[bee(pk)]
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub id: i64,
    /// 凭据无效/未带 token 时为 0
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub admin_id: i64,
    #[bee(sql_type = "VARCHAR(64)")]
    pub username: String,
    /// 模块码：admin / role / menu / dept / dict / loginlog / auditlog / auth / other
    #[bee(sql_type = "VARCHAR(32)")]
    pub module: String,
    /// 动作中文名，如「新增管理员」
    #[bee(sql_type = "VARCHAR(64)")]
    pub action: String,
    #[bee(sql_type = "VARCHAR(10)")]
    pub method: String,
    #[bee(sql_type = "VARCHAR(255)")]
    pub path: String,
    /// 1 成功（2xx） / 0 失败
    pub status: i8,
    /// 失败原因摘要；成功为空
    #[bee(sql_type = "VARCHAR(255)")]
    pub msg: String,
    pub duration_ms: i64,
    #[bee(sql_type = "VARCHAR(45)")]
    pub ip: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
}
