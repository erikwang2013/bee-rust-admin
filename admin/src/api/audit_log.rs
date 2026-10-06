// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{csv, page_size};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppQuery, ok};
use crate::models::AuditLog;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct AuditListQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub username: Option<String>,
    pub module: Option<String>,
    pub status: Option<i8>,
    /// "YYYY-MM-DD HH:MM:SS"
    pub start: Option<String>,
    pub end: Option<String>,
}

/// 组装筛选条件（不含数据权限），列表/清空/导出共用。
fn filtered(
    q: &AuditListQuery,
    scope: &datascope::DataScope,
) -> Result<bee_orm::QuerySet<AuditLog>, ApiError> {
    let mut qs = AuditLog::query();
    if let Some(u) = q.username.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("username", u.trim()).map_err(ApiError::from)?;
    }
    if let Some(m) = q.module.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_eq("module", m.trim()).map_err(ApiError::from)?;
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
    // 与登录记录同规则：非超管按数据权限看（部门 / 仅本人）
    if let Some((sql, params)) = scope.admin_id_condition() {
        qs = qs.filter_raw(sql, &params);
    }
    Ok(qs)
}

pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<AuditListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:auditlog:list")?;
    let (page, size) = page_size(q.page, q.size);
    let scope = datascope::resolve(&auth, &state.db).await?;
    let (rows, total) = filtered(&q, &scope)?
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

/// 按当前筛选条件清空。自带审计：这次删除本身也会被中间件记一条。
pub async fn clear(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<AuditListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:auditlog:remove")?;
    let scope = datascope::resolve(&auth, &state.db).await?;
    let rows = filtered(&q, &scope)?
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;
    let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
    let deleted = crate::api::delete_by_ids(&state, "audit_log", &ids).await?;
    Ok(ok(json!({ "deleted": deleted })))
}

/// 导出当前筛选结果（不含分页）；鉴权复用 list 权限码。
pub async fn export(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<AuditListQuery>,
) -> Result<Response, ApiError> {
    auth.require("system:auditlog:list")?;
    let scope = datascope::resolve(&auth, &state.db).await?;
    let rows = filtered(&q, &scope)?
        .order_by("id DESC")
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;

    let mut out = csv::row(&[
        "ID", "时间", "用户", "模块", "动作", "方法", "路径", "结果", "详情", "耗时(ms)", "IP",
    ]
    .map(String::from)
    .to_vec());
    for r in rows {
        out.push_str(&csv::row(&[
            r.id.to_string(),
            r.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            r.username,
            r.module,
            r.action,
            r.method,
            r.path,
            if r.status == 1 { "成功".into() } else { "失败".into() },
            r.msg,
            r.duration_ms.to_string(),
            r.ip,
        ]));
    }
    Ok(csv::response(format!("audit-logs-{}.csv", csv::today()), out))
}
