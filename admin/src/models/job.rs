// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 定时任务。任务体在 `crate::jobs` 里代码注册（不落库：界面能写脚本就是 RCE），
/// 这张表只管调度参数（间隔/开关）与上次执行结果。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "job", pk = "id")]
pub struct Job {
    #[bee(auto)]
    pub id: u64,
    #[bee(len = 64)]
    pub name: String,
    /// 代码注册键：`jobs::JOBS` 里没有的 code 不可手动触发（409）
    #[bee(unique)]
    #[bee(len = 64)]
    pub code: String,
    /// **间隔秒数**（如 "3600"）。字段名保留 cron 是为了将来换表达式时不改列名；
    /// 现在不做表达式解析：内置任务都是「每小时/每天跑一次」，间隔语义足够。
    #[bee(len = 64)]
    pub cron: String,
    /// 1 启用 / 0 停用
    pub status: i8,
    /// 上次执行开始时间；调度循环用它判到期 + 抢占
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    pub last_run_at: Option<NaiveDateTime>,
    /// 1 成功 / 0 失败；没跑过为 NULL
    pub last_status: Option<i8>,
    #[bee(len = 255)]
    pub last_msg: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}

/// 任务执行记录（手动触发与调度循环都写）。时间列叫 `started_at`；
/// `job_code` 的复合索引 `idx_code_time(job_code, started_at)` 走 `seed::migrate` 的裸 DDL
/// （syncdb 只建单列索引）。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "job_log", pk = "id")]
pub struct JobLog {
    #[bee(auto)]
    pub id: u64,
    #[bee(index)]
    #[bee(len = 64)]
    pub job_code: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub started_at: NaiveDateTime,
    pub duration_ms: u64,
    /// 1 成功 / 0 失败
    pub status: i8,
    #[bee(len = 255)]
    pub msg: String,
}
