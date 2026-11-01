// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! v2 列表 / 未读的摘要行：不带正文，正文按详情接口取。
use crate::models::Notice;
use bee_orm::{OrmError, Row};
use chrono::NaiveDateTime;
use serde::Serialize;

/// 列表 / 未读行：**不带正文**。正文最长 20k 字（64KB），列表最多 100 行、
/// 铃铛 50 行，整篇下发既费带宽又没必要 —— 编辑和「查看全文」按需走详情。
#[derive(Serialize)]
pub(super) struct NoticeSummary {
    #[serde(serialize_with = "crate::hid::ser_id")]
    id: i64,
    title: String,
    status: i8,
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    published_at: Option<NaiveDateTime>,
    #[serde(serialize_with = "crate::util::ser_dt")]
    created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    updated_at: NaiveDateTime,
}

impl NoticeSummary {
    /// 未读列表走裸 SQL 只查摘要列，连库里的正文都不读。
    pub(super) fn from_row(row: &Row) -> Result<Self, OrmError> {
        Ok(Self {
            id: bee_orm::decode(row, "id")?,
            title: bee_orm::decode(row, "title")?,
            status: bee_orm::decode(row, "status")?,
            published_at: bee_orm::decode(row, "published_at")?,
            created_at: bee_orm::decode(row, "created_at")?,
            updated_at: bee_orm::decode(row, "updated_at")?,
        })
    }
}

impl From<Notice> for NoticeSummary {
    fn from(n: Notice) -> Self {
        Self {
            id: n.id,
            title: n.title,
            status: n.status,
            published_at: n.published_at,
            created_at: n.created_at,
            updated_at: n.updated_at,
        }
    }
}
