// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 定时任务（v1.6.0 C2a）：列表 / 改间隔与开关 / 手动触发 / 执行记录。
//! 任务体不落库（代码注册，见 `crate::jobs`），所以这里没有增删接口。
use crate::api::{PagingExt, dt_ge, dt_le, page_size};
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::hid;
use crate::jobs;
use crate::models::{Job, JobLog};
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use bee_orm::Model;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct JobQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub name: Option<String>,
    pub status: Option<i8>,
}

/// 名字与 code 不可改（代码注册：改了名字无所谓，改了 code 就等于换了个任务）——
/// 更新体里干脆没有这两个字段。间隔是**秒数**，必须能解析成正整数。
#[derive(Deserialize)]
pub struct JobUpdate {
    pub cron: String,
    pub status: i8,
}

/// 间隔秒数校验：非数字 / 0 / 负数一律 400。
/// 0 会让任务每个 tick 都跑，所以不接受（要停就置 status=0）。
fn check_cron(cron: &str) -> Result<String, ApiError> {
    let s = cron.trim();
    match s.parse::<i64>() {
        Ok(n) if n >= 1 => Ok(n.to_string()),
        _ => Err(ApiError::BadRequest("间隔必须是正整数秒".into())),
    }
}

#[apidoc::title("定时任务列表")]
#[apidoc::desc("含上次执行结果与下次执行时间")]
#[apidoc::url("/api/v1/jobs")]
#[apidoc::method("GET")]
#[apidoc::tag("任务")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<JobQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:job:list")?;
    let (page, size) = page_size(q.page, q.size);
    let mut qs = Job::query();
    if let Some(n) = q.name.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("name", n.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    // id ASC = 代码注册表顺序（内置任务的清单，稳定比「最新在前」有用）
    let (rows, total) = qs
        .order_by("id ASC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

#[apidoc::title("编辑定时任务")]
#[apidoc::desc("间隔秒数非数字 / 0 / 负数一律 400（0 会让任务每个 tick 都跑；要停就置 status = 0）")]
#[apidoc::url("/api/v1/jobs/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("任务")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "任务对外 id（hashids 短串）")]
#[apidoc::param(name = "cron", ty = "string", desc = "cron 表达式（与 interval_secs 二选一）")]
#[apidoc::param(name = "interval_secs", ty = "int", desc = "间隔秒数，必须 > 0")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 停用")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<JobUpdate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:job:edit")?;
    let id = hid::dec(&id)?;
    let cron = check_cron(&body.cron)?;

    let mut job = Job::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    job.cron = cron;
    job.status = body.status;
    job.updated_at = now();
    job.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

/// 手动触发一次：**后台执行**，接口立即返回（不再等任务跑完）。
/// 跑完的结果写 `job_log` 并回写 job 行，前端刷新列表 / 执行记录看；
/// 任务自己失败不算接口失败（执行记录里是 status: 0 + 原因）。
///
/// 为什么不等：全局请求超时是 30 秒（`bee_router` 的 TimeoutLayer），
/// mysqldump、大导出这些任务跑得比它久 —— 同步执行会被掐断成 408，
/// 而且掐断的是 handler 的 future：任务半途而废，`job_log` 无行、`.part` 孤儿。
#[apidoc::title("手动触发一次任务")]
#[apidoc::desc("**后台执行、立即返回**（不阻塞等跑完）：结果写 job_log 并回写任务行；已在执行中回 409")]
#[apidoc::url("/api/v1/jobs/{id}/run")]
#[apidoc::method("POST")]
#[apidoc::tag("任务")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "任务对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("409")]
#[apidoc::not_debug]
pub async fn run(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:job:edit")?;
    let id = hid::dec(&id)?;
    let job = Job::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    // 库里残留的旧 code（任务已从代码里下线）：不可手动触发，避免点到不认识的任务
    let Some(spec) = jobs::spec(&job.code) else {
        return Err(ApiError::job_not_registered(&job.code));
    };
    // acquire-first：先在**当前请求**里拿闸，拿不到当场 409；拿到才 spawn，permit 随
    // 任务走。旧写法「先 is_running 预检、spawn 里再 acquire」有毫秒级窗口 —— 两个
    // 请求都过预检时，后一个的任务体在 jobs::run 里拿不到闸、静默跳过，而接口已经
    // 回了 200（用户看到「已触发」却没执行）。现在「受理」与「占住闸」是同一个动作。
    let Some(permit) = jobs::Busy::acquire(spec.code) else {
        return Err(ApiError::job_running(&job.code));
    };
    tokio::spawn(async move {
        jobs::run_holding(&state, job, spec, permit).await;
    });
    Ok(ok(json!({ "triggered": true })))
}

#[derive(Deserialize)]
pub struct JobLogQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub job_code: Option<String>,
    pub status: Option<i8>,
    /// "YYYY-MM-DD HH:MM:SS"，作用在 started_at 上
    pub start: Option<String>,
    pub end: Option<String>,
}

#[apidoc::title("任务执行日志")]
#[apidoc::desc("按任务 / 状态筛选 + 分页")]
#[apidoc::url("/api/v1/job-logs")]
#[apidoc::method("GET")]
#[apidoc::tag("任务")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100")]
#[apidoc::query(name = "job_id", ty = "string", desc = "任务短串（只看某个任务的记录）")]
#[apidoc::query(name = "status", ty = "int", desc = "执行结果：1 成功 / 0 失败")]
#[apidoc::response_status("200")]
pub async fn log_list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<JobLogQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:job:list")?;
    let (page, size) = page_size(q.page, q.size);
    let mut qs = JobLog::query();
    if let Some(c) = q.job_code.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_eq("job_code", c.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    // 区间端点是闭区间：上游只有 `>` / `<`，`dt_ge`/`dt_le` 把端点挪 1 秒换等价（值仍走绑定）
    if let Some(s) = q.start.as_deref().filter(|s| !s.is_empty()) {
        qs = qs.filter_gt("started_at", dt_ge(s)).map_err(ApiError::from)?;
    }
    if let Some(e) = q.end.as_deref().filter(|s| !s.is_empty()) {
        qs = qs.filter_lt("started_at", dt_le(e)).map_err(ApiError::from)?;
    }
    let (rows, total) = qs
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_must_be_positive_integer() {
        assert_eq!(check_cron("3600").unwrap(), "3600");
        assert_eq!(check_cron(" 2 ").unwrap(), "2");
        for bad in ["", "abc", "0", "-5", "3.5", "1h"] {
            assert!(check_cron(bad).is_err(), "{bad:?} 必须拒绝");
        }
    }
}
