// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 定时任务（v1.6.0 C2a）：列表 / 改间隔与开关 / 手动触发 / 执行记录。
//! 任务体不落库（代码注册，见 `crate::jobs`），所以这里没有增删接口。
use crate::api::{PagingExt, dt_ge, dt_le, page_size};
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
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

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<i64>,
    AppJson(body): AppJson<JobUpdate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:job:edit")?;
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

/// 手动触发一次：**同步执行**，结果写 `job_log` 并回写 job 行。
/// 任务自己失败（返回 Err）不算接口失败：回 200 + `status: 0`，前端看 `msg` 提示。
pub async fn run(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<i64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:job:edit")?;
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
    let (status, msg, duration_ms) = jobs::run(&state, job, spec).await;
    Ok(ok(json!({ "status": status, "msg": msg, "duration_ms": duration_ms })))
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
