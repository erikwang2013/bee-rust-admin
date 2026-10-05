// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::page_size;
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, ok};
use crate::models::LoginLog;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
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
    if let Some((sql, params)) = scope.login_log_condition() {
        qs = qs.filter_raw(sql, &params);
    }
    Ok(qs)
}

pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<LogListQuery>,
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
/// ponytail: 先取 id 再分批删（ORM 无按条件批量删）；日志量极大时改为 ORM 批量删。
pub async fn clear(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<LogListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:loginlog:remove")?;
    let scope = datascope::resolve(&auth, &state.db).await?;
    // 分页参数对「全量筛选」不生效：fetch_all 拿全部匹配 id
    let rows = filtered(&q, &scope)?
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;
    let mut deleted = 0u64;
    for chunk in rows.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        // id 来自数据库，非用户输入；表名写死
        let sql = format!("DELETE FROM login_log WHERE id IN ({placeholders})");
        let mut qs = sqlx::query(&sql);
        for r in chunk {
            qs = qs.bind(r.id);
        }
        qs.execute(state.db.pool())
            .await
            .map_err(|e| ApiError::from(bee_orm::OrmError::from(e)))?;
        deleted += chunk.len() as u64;
    }
    Ok(ok(json!({ "deleted": deleted })))
}
