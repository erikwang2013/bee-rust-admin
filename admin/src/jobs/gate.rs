// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 进程内「已在跑」闸与任务执行体。

use super::*;

/// 同进程内的「已在跑」闸：手动触发改成**后台执行**后（api::job::run 里 spawn），
/// 光靠 `last_run_at` 拦不住并发 —— 它只在调度 tick 内部串行有效，连点几次触发就是
/// 几份同时跑（db_backup 叠出多份 mysqldump）。粒度是 code：同一个任务不并发，
/// 不同任务互不影响。shortcut: 只在单进程内生效（与调度同一前提），多实例部署要换成 DB 行锁。
static RUNNING: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

/// 占位凭据：执行全程持有，Drop 时释放 —— 正常返回、提前 return、panic 都不会漏放。
/// 手动触发的接口在 **spawn 之前** 调 [`Busy::acquire`]（失败当场 409），把 permit
/// 交给 [`run_holding`]；调度循环走 [`run`]（内部自己拿）。
pub struct Busy(&'static str);
impl Busy {
    pub fn acquire(code: &'static str) -> Option<Self> {
        let mut v = RUNNING.lock().ok()?;
        if v.contains(&code) {
            return None;
        }
        v.push(code);
        Some(Self(code))
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        if let Ok(mut v) = RUNNING.lock() {
            v.retain(|c| *c != self.0);
        }
    }
}

/// 调度循环的入口：自己拿闸，拿不到就跳过（不写 job_log、不动 `last_run_at`：
/// 没执行就不是一次执行记录）。
pub async fn run(state: &AppState, job: Job, spec: &JobSpec) -> (i8, String, i64) {
    let Some(permit) = Busy::acquire(spec.code) else {
        return (0, format!("跳过：任务 {} 上一次还在执行中", spec.code), 0);
    };
    run_holding(state, job, spec, permit).await
}

/// 执行一个任务（**调用方已持 permit**）：抢占 → 跑 → 写 `job_log` → 回写 job 行。
/// 手动触发在接口层先 [`Busy::acquire`]，拿到才 spawn 到这里 —— 这样「受理」与
/// 「占住闸」是同一个同步动作，不存在「接口回 200、任务体却静默跳过」的窗口。
/// permit 随任务走到函数结束（Drop 释放）。
pub async fn run_holding(
    state: &AppState,
    job: Job,
    spec: &JobSpec,
    _permit: Busy,
) -> (i8, String, i64) {
    let started = now();
    // 抢占：先把 last_run_at 落库，同一进程的下一 tick 不会再选它
    let mut claimed = job.clone();
    claimed.last_run_at = Some(started);
    claimed.updated_at = started;
    if let Err(e) = claimed.update(&state.db).await {
        tracing::warn!("抢占任务 {} 失败: {e}", job.code);
    }

    let t0 = std::time::Instant::now();
    let (status, msg) = match (spec.run)(state.clone()).await {
        Ok(m) => (1i8, m),
        Err(e) => (0i8, e),
    };
    let duration_ms = t0.elapsed().as_millis() as i64;
    // 手工摘要/错误信息可能带换行或超长：列是 varchar(255)
    let msg: String = msg.replace(['\r', '\n'], " ").chars().take(255).collect();

    // 记录写失败不能影响任务本身（发号失败也一样：跳过这条记录）
    match state.next_id() {
        Ok(id) => {
            let row = JobLog {
                id,
                job_code: job.code.clone(),
                started_at: started,
                duration_ms,
                status,
                msg: msg.clone(),
            };
            if let Err(e) = row.insert(&state.db).await {
                tracing::warn!("写任务执行记录失败: {e}");
            }
        }
        Err(e) => tracing::warn!("生成任务记录 id 失败: {e:?}"),
    }

    let mut done = claimed;
    done.last_status = Some(status);
    done.last_msg = msg.clone();
    done.updated_at = now();
    if let Err(e) = done.update(&state.db).await {
        tracing::warn!("回写任务 {} 状态失败: {e}", job.code);
    }
    (status, msg, duration_ms)
}
