// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 异步导出任务（项 4）。「导什么」与取数函数是**代码注册**的（`crate::export::KINDS`，
/// 与定时任务同一套样式：能往库里写导出语句就是任意读库），这张表只放
/// 「谁要的 / 什么筛选条件 / 跑到哪了 / 文件在哪 / 什么时候过期」。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "export_task")]
pub struct ExportTask {
    #[bee(pk)]
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub id: i64,
    /// 发起人：也是「谁能看 / 下 / 删」的判据（导出中心只列自己的）
    pub admin_id: i64,
    /// 发起人用户名（列表直接显示，不为一个名字再联表）
    #[bee(sql_type = "VARCHAR(64)")]
    pub username: String,
    /// 代码注册键（`KINDS` 里没有的 = 任务失败，与 job.code 同一口径）
    #[bee(sql_type = "VARCHAR(32)")]
    pub kind: String,
    /// 筛选条件 JSON（字段与同步导出的 query 一致，创建时按同一份结构校验过）
    #[bee(sql_type = "VARCHAR(1024)")]
    pub params: String,
    /// 0 待处理 / 1 执行中 / 2 已完成 / 3 失败 / 4 已取消
    pub status: i8,
    /// 落盘绝对路径；清理任务删记录时**连着它一起删**（谁也不会留下孤儿文件）
    #[bee(sql_type = "VARCHAR(255)")]
    pub file_path: String,
    pub row_count: i64,
    pub file_size: i64,
    /// 已写进文件的行数：执行中每批（5000 行）回写一次，列表上就是「已经导到哪了」。
    /// 终态时与 `row_count` 同值（取消的那行是取消那一刻的部分值，见 `api::export::cancel`）
    pub progress_rows: i64,
    /// 失败原因 / 完成摘要
    #[bee(sql_type = "VARCHAR(255)")]
    pub msg: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    /// 认领时刻（执行中的行才有）
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    pub started_at: Option<NaiveDateTime>,
    /// 心跳：每批回写进度时一起刷新，是「这个任务还活着」的唯一证据。
    /// 判「进程崩过要重跑」与收残留 `.part` 都看它（见 `last_seen`）——只看认领时刻的话，
    /// 跑得久的好任务会被误判成死任务（M1）
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    pub heartbeat_at: Option<NaiveDateTime>,
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    pub finished_at: Option<NaiveDateTime>,
    /// 过期时刻（创建时 = 现在 + `[export] retain_days`）：清理任务按它删
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub expire_at: NaiveDateTime,
}

impl ExportTask {
    /// 最后一次「还活着」的证据：心跳优先，没见过心跳才回落到认领时刻。
    /// 判「进程崩过的行」重跑与收残留 `.part` **共用这一个函数**——两处各写一遍
    /// `COALESCE` 迟早漂开（一处看心跳、一处看认领时刻，行为就对不上了）。
    pub fn last_seen(&self) -> Option<NaiveDateTime> {
        self.heartbeat_at.or(self.started_at)
    }
}
