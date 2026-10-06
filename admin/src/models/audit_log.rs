// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 操作日志（审计）：中间件对写操作自动落一条。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "audit_log", pk = "id")]
pub struct AuditLog {
    #[bee(auto)]
    pub id: u64,
    /// 凭据无效/未带 token 时为 0
    pub admin_id: u64,
    #[bee(len = 64)]
    pub username: String,
    /// 模块码：admin / role / menu / dept / dict / loginlog / auditlog / auth / other
    #[bee(len = 32)]
    pub module: String,
    /// 动作中文名，如「新增管理员」
    #[bee(len = 64)]
    pub action: String,
    #[bee(len = 10)]
    pub method: String,
    #[bee(len = 255)]
    pub path: String,
    /// 1 成功（2xx） / 0 失败
    pub status: i8,
    /// 失败原因摘要；成功为空
    pub msg: String,
    pub duration_ms: u64,
    #[bee(len = 45)]
    pub ip: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
}
