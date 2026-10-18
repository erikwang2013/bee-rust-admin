// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// Webhook 投递失败账本（F4-3）：**成功不落行**，只记失败与终态。
/// `payload` 是实际发出的 JSON 体原文（重投原样再发；签名每投按当前时间戳现算）。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "notify_delivery")]
pub struct NotifyDelivery {
    #[bee(pk)]
    pub id: i64,
    #[bee(sql_type = "VARCHAR(64)")]
    pub event: String,
    #[bee(sql_type = "TEXT")]
    pub payload: String,
    /// `pending` / `delivered` / `failed`
    #[bee(sql_type = "VARCHAR(16)")]
    pub status: String,
    /// 已尝试次数：首次失败 = 1，每重投一次 +1（上限 4 = 首发 + 3 次重投）
    pub attempts: i32,
    /// 下次重投时间；终态（delivered / failed）为 NULL
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    pub next_at: Option<NaiveDateTime>,
    #[bee(sql_type = "VARCHAR(255)")]
    pub msg: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
}
