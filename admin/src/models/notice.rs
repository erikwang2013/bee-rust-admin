// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 通知公告。已读关系在 `notice_read`（复合主键，建表走裸 DDL，本模块按裸 SQL 读写）。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "notice")]
pub struct Notice {
    #[bee(pk, auto)]
    pub id: i64,
    #[bee(sql_type = "VARCHAR(128)")]
    pub title: String,
    #[bee(sql_type = "TEXT")]
    pub content: String,
    /// 0 草稿 / 1 已发布：只有已发布的进未读集合
    pub status: i8,
    pub created_by: i64,
    /// 首次置 1 时写；改回 0 不清空（留痕：曾发布过）
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    pub published_at: Option<NaiveDateTime>,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
