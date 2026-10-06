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

fn header_cells() -> Vec<String> {
    ["ID", "时间", "用户", "模块", "动作", "方法", "路径", "结果", "详情", "耗时(ms)", "IP"]
        .map(String::from)
        .to_vec()
}

/// 取一批导出数据（keyset：`id < last`，首页不带该条件），行渲染与旧的一次性实现逐字段一致。
async fn export_batch(
    state: &AppState,
    base: &bee_orm::QuerySet<AuditLog>,
    last: Option<u64>,
) -> Result<Vec<(u64, String)>, ApiError> {
    let mut qs = base.clone().order_by("id DESC").limit(csv::BATCH);
    if let Some(last) = last {
        qs = qs.filter_raw("id < ?", &[last]);
    }
    let rows = qs.fetch_all(&state.db).await.map_err(ApiError::from)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.id,
                csv::row(&[
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
                ]),
            )
        })
        .collect())
}

/// 导出当前筛选结果（不含分页）；鉴权复用 list 权限码。
/// 流式：筛选条件与 list 共用 `filtered()`，按 keyset 分批取（B6），内存只与一批成正比。
pub async fn export(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<AuditListQuery>,
) -> Result<Response, ApiError> {
    auth.require("system:auditlog:list")?;
    let scope = datascope::resolve(&auth, &state.db).await?;
    let base = filtered(&q, &scope)?;
    csv::streamed(format!("audit-logs-{}.csv", csv::today()), header_cells(), move |last| {
        let state = state.clone();
        let base = base.clone();
        async move { export_batch(&state, &base, last).await }
    })
    .await
}
