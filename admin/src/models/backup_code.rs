// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// TOTP 备用码（F4-1）：每行一枚，只存 SHA-256 哈希 + 服务端 pepper（见 `crate::totp`）。
/// `used_at` 非空 = 已消费；重新生成时旧行整批删除（不是标记作废，避免越攒越多）。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "admin_backup_code")]
pub struct AdminBackupCode {
    #[bee(pk)]
    pub id: i64,
    pub admin_id: i64,
    /// SHA-256(pepper ‖ code) 的小写 hex（64 字符）。明文只在生成响应里出现一次。
    #[bee(sql_type = "VARCHAR(64)")]
    pub code_hash: String,
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    pub used_at: Option<NaiveDateTime>,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
}
