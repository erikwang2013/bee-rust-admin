// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{csv, page_size};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppQuery, ok};
use crate::models::LoginLog;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct LogListQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub username: Option<String>,
    pub status: Option<i8>,
    /// "YYYY-MM-DD HH:MM:SS"
    pub start: Option<String>,
    pub end: Option<String>,
}

/// 组装筛选条件（不含数据权限），列表与清空共用。
fn filtered(q: &LogListQuery, scope: &datascope::DataScope) -> Result<bee_orm::QuerySet<LoginLog>, ApiError> {
    let mut qs = LoginLog::query();
    if let Some(u) = q.username.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("username", u.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    if let Some(s) = q.start.as_deref().filter(|s| !s.is_empty()) {
        qs = qs.filter_raw("created_at >= ?", &[s]);
    }
    if let Some(e) = q.end.as_deref().filter(|s| !s.is_empty()) {
        qs = qs.filter_raw("created_at <= ?", &[e]);
    }
    if let Some((sql, params)) = scope.admin_id_condition() {
        qs = qs.filter_raw(sql, &params);
    }
    Ok(qs)
}

pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<LogListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:loginlog:list")?;
    let (page, size) = page_size(q.page, q.size);
    let scope = datascope::resolve(&auth, &state.db).await?;
    let (rows, total) = filtered(&q, &scope)?
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

/// 按当前筛选条件清空。
/// ponytail: 先全量取 id 再分批删（ORM 无按条件批量删）；日志量极大时改为 ORM 批量删。
pub async fn clear(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<LogListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:loginlog:remove")?;
    let scope = datascope::resolve(&auth, &state.db).await?;
    // 分页参数对「全量筛选」不生效：fetch_all 拿全部匹配 id
    let rows = filtered(&q, &scope)?
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;
    let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
    let deleted = crate::api::delete_by_ids(&state, "login_log", &ids).await?;
    Ok(ok(json!({ "deleted": deleted })))
}

/// 导出当前筛选结果（不含分页）；鉴权复用 list 权限码。
/// ponytail: 一次全量进内存，十万行级没问题；再大要走流式导出。
pub async fn export(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<LogListQuery>,
) -> Result<Response, ApiError> {
    auth.require("system:loginlog:list")?;
    let scope = datascope::resolve(&auth, &state.db).await?;
    let rows = filtered(&q, &scope)?
        .order_by("id DESC")
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;

    let mut out = csv::row(&header_cells());
    for r in rows {
        out.push_str(&csv::row(&[
            r.id.to_string(),
            r.username,
            r.ip,
            r.user_agent,
            if r.status == 1 { "成功".into() } else { "失败".into() },
            r.msg,
            r.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        ]));
    }
    Ok(csv::response(format!("login-logs-{}.csv", csv::today()), out))
}

fn header_cells() -> Vec<String> {
    ["ID", "用户名", "IP", "User-Agent", "结果", "详情", "时间"]
        .map(String::from)
        .to_vec()
}
