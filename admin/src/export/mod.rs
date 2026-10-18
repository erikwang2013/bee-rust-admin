// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 异步导出（项 4）：导出**类型**的代码注册表（`KINDS`）+ 执行/清理两个任务体。
//!
//! 口径：
//! - 每个 kind 的 CSV 表头与取数函数**直接复用同步导出的那份**（`api::audit_log` /
//!   `api::login_log` 里的 `headers()` / `export_batch()`）——同一个文件 `/xxx/export`
//!   与这里的异步文件逐字节一致，没有第二套渲染逻辑可变。
//! - 执行挂在既有 job 循环（`JOBS` 注册，样式与项 9 的 db_backup 一致）：
//!   `export_run` 每分钟把待处理任务跑掉，`export_clean` 每天清过期（记录 + 文件）。
//! - 数据权限在**执行时**按发起人**当前**的角色重算（`datascope::from_roles`），不是发起时
//!   快照：权限被收窄后，还没跑完的任务不会把已经看不到的数据导出去；发起人被删 → 任务失败。
//! - 文件落在 `{upload_dir}/exports/{id}.{代次}.csv`（先写同代次的 `.part` 再改名，
//!   中断不留半份）。代次 = 认领时写的 `started_at`：同一行被重新认领后，新旧执行者的
//!   `.part` 与成品都互不覆盖（N1：输家的改名也盖不掉赢家），输家的 `PartFile`
//!   只会删自己那份；过期清理按名字约定收走无人引用的旧代次成品。
//! - 任务可以**取消**（行置 4）：执行者每批回写进度时顺带探一次（谓词 `status = 1` 且
//!   `started_at = 本代次` 命中 0 行 = 被取消或被新执行者接管），批间收手、半份由
//!   `PartFile` 清掉；同一批 5000 行内不多停。收尾那一写也用同一把 fencing token
//!   （见 `finish`）：没写进去时**先读行的现状**——只有已取消（4）或行已被删才撤成品，
//!   写库失败/被接管都把文件留给赢家（M1：不能出现「status=2 却没有文件」）。
//! - 进度与**心跳**一起回写（`progress_rows` / `heartbeat_at`）：判「进程崩过」与收残留
//!   `.part` 都看心跳（`last_seen`），长任务因此不会被误判成死任务重跑（M1）。
//! - 任务是**并发派发**的（`tokio::spawn` + **进程级共享**信号量 `EXPORT_CONCURRENCY`），
//!   `run()` 派发完就返回，不占住 job 循环；跨 tick / 多次手动触发共用同一个闸，不会
//!   叠加出超过 2 个同时执行的任务；同一行仍由认领排他保证只有一个执行者。

use crate::api::{audit_log, csv, login_log};
use crate::datascope::{self, DataScope};
use crate::error::ApiError;
use crate::models::{Admin, ExportTask, Role};
use crate::state::AppState;
use crate::util::now;
use crate::relations::RelationsExt;
use bee_orm::Value;
use chrono::NaiveDateTime;
use serde_json::Value as JsonVal;
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

mod clean;
mod kinds;
mod run;

#[cfg(test)]
mod tests;

pub use clean::clean;
pub use kinds::{dir, spec};
pub use run::run;

/// 认领超时 / 心跳超时：执行中的行超过这么久**没有心跳**（从没心跳过就看认领时刻），
/// 当成「进程崩过」重跑一次。心跳由每个 5000 行批回写（`beat`），只要还在跑就一直有 ——
/// 长任务不会再被误判成死任务重跑一遍（M1）。
/// shortcut: 固定 10 分钟，真要按任务类型分档再进配置（导出都是分钟级，够用）
const STALE_CLAIM_SECS: i64 = 600;
