// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 定时任务（v1.6.0 C2a）：任务体**代码注册**（`JOBS`），数据库只管调度与执行记录。
//!
//! 调度语义：
//! - `job.cron` 存的是**间隔秒数**（列名保留 cron 是为了将来换表达式时不改列名）；
//!   不做 cron 表达式解析：内置任务都是「每小时/每天跑一次」，间隔语义足够。
//! - 循环每 10 秒 tick 一次，选出 `status=1` 且 `last_run_at` 为空或已过间隔的任务
//!   **串行**执行（任务少，并发带来的写冲突不值得）。
//! - 同一进程内用 `last_run_at` **抢占**：执行前先落库，tick 重叠时不会再选它。
//!   ponytail: 单实例假设，多实例部署要换成 DB 行锁（本项目的 admin 是单实例）。
//! - `[job] enabled=false` 时**不启动循环**（手动触发仍可用：运维要能临时关掉定时、手动补跑）。

mod gate;
mod retention;

#[cfg(test)]
mod tests;

pub use gate::{Busy, run, run_holding};

use retention::{attach_orphan_clean, avatar_orphan_clean, log_retention};
use crate::models::{Admin, Attachment, Job, JobLog};
use crate::state::AppState;
use bee_orm::Model;
use crate::util::now;
use std::time::Duration;

type JobFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>;

/// 一条代码注册的任务。`run` 收 `AppState`（内部都是 Arc，克隆廉价）。
pub struct JobSpec {
    pub code: &'static str,
    pub name: &'static str,
    /// 首次 seed 写进 `job.cron` 的默认间隔秒数；之后以库里的值为准
    pub interval_secs: i64,
    /// 返回的字符串进 `job_log.msg`
    pub run: fn(AppState) -> JobFuture,
}

pub const JOBS: &[JobSpec] = &[
    JobSpec {
        code: "log_retention",
        name: "日志清理",
        interval_secs: 86400,
        run: |s| Box::pin(log_retention(s)),
    },
    JobSpec {
        code: "avatar_orphan_clean",
        name: "头像孤儿清理",
        interval_secs: 86400,
        run: |s| Box::pin(avatar_orphan_clean(s)),
    },
    // 附件孤儿清理：每小时一趟 —— 配额是**行级**口径（删行立刻还配额），磁盘回收只靠它，
    // 收早也不怕：保护期 300s（ATTACH_MIN_AGE_SECS）远小于一小时，不会误收「先落盘后入库」的成品
    JobSpec {
        code: "attach_orphan_clean",
        name: "附件孤儿清理",
        interval_secs: 3600,
        run: |s| Box::pin(attach_orphan_clean(s)),
    },
    // 每天一份、保留 7 份（[backup] keep）：默认节奏是一周内的回滚窗口。
    // 备份本体在 crate::backup（解析 DSN → mysqldump → 落盘 → 清理）
    JobSpec {
        code: "db_backup",
        name: "数据库备份",
        interval_secs: 86400,
        run: |s| Box::pin(crate::backup::run(s)),
    },
    // 异步导出（项 4）：执行体在 crate::export。每分钟一趟，发起后最迟一分钟跑完；
    // 单独一个清理任务（每天）删过期文件与记录
    JobSpec {
        code: "export_run",
        name: "导出任务执行",
        interval_secs: 60,
        run: |s| Box::pin(crate::export::run(s)),
    },
    JobSpec {
        code: "export_clean",
        name: "导出文件清理",
        interval_secs: 86400,
        run: |s| Box::pin(crate::export::clean(s)),
    },
    // 登录设备清理（E2-2）：清「已踢且 [auth] device_keep_days 天没露面」的行，
    // 执行体在 crate::retention（口径与安全线写在那边）
    JobSpec {
        code: "device_clean",
        name: "登录设备清理",
        interval_secs: 86400,
        run: |s| Box::pin(crate::retention::device_clean(s)),
    },
    // Webhook 重投（F4-3）：每 5 分钟取 due 的失败记录重发（1/5/15 分钟退避，
    // 最多 3 次），顺带清理 30 天前的账本行。执行体在 crate::notify
    JobSpec {
        code: "notify_retry",
        name: "Webhook 失败重投",
        interval_secs: 300,
        run: |s| Box::pin(crate::notify::notify_retry(s)),
    },
];

/// 按 code 找注册项；库里残留的旧 code 找不到 = 不可手动触发（409）、调度跳过。
pub fn spec(code: &str) -> Option<&'static JobSpec> {
    JOBS.iter().find(|j| j.code == code)
}

/// 调度循环的 tick 间隔。
const TICK: Duration = Duration::from_secs(10);

/// 调度循环：每 10 秒选一次到期任务串行跑。由 main 在 `[job] enabled` 时 spawn。
pub async fn scheduler(state: AppState) {
    let mut tick = tokio::time::interval(TICK);
    // 任务跑超一个 tick 时不要补跑积压的 tick（默认 Burst 会连着触发一串）
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        tick_once(&state).await;
    }
}

/// 一个 tick：把到期的任务串行跑一遍。查库失败只告警，下一个 tick 再试。
async fn tick_once(state: &AppState) {
    let qs = match Job::query().filter_eq("status", 1) {
        Ok(qs) => qs,
        Err(e) => return tracing::warn!("构造任务查询失败: {e}"),
    };
    let jobs = match qs.all(&state.db).await {
        Ok(v) => v,
        Err(e) => return tracing::warn!("查询到期任务失败: {e}"),
    };
    for job in jobs {
        if !due(&job, now()) {
            continue;
        }
        // 库里残留的旧 code：跳过（不报错——它只是不会再被执行，列表里仍看得到）
        let Some(spec) = spec(&job.code) else {
            tracing::debug!("任务 {} 不在代码注册表里，跳过", job.code);
            continue;
        };
        run(state, job, spec).await;
    }
}

/// `cron`（间隔秒数）→ 秒数。0 / 非数字（手工改库改坏）→ None = 永不到期。
pub(super) fn interval_secs(cron: &str) -> Option<i64> {
    cron.trim().parse::<i64>().ok().filter(|s| *s > 0)
}

/// 到期判定：没跑过（NULL）= 到期；否则 `now - last_run_at >= 间隔`。
pub(super) fn due(job: &Job, now: chrono::NaiveDateTime) -> bool {
    let Some(interval) = interval_secs(&job.cron) else {
        return false;
    };
    match job.last_run_at {
        None => true,
        Some(last) => now >= last + chrono::Duration::seconds(interval as i64),
    }
}
