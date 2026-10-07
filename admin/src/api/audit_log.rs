// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{PagingExt, csv, dt_ge, dt_le, page_size};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppQuery, ok};
use crate::hid;
use crate::models::AuditLog;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, Clone)]
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
    // 区间端点是闭区间：上游只有 `>` / `<`，`dt_ge`/`dt_le` 把端点挪 1 秒换等价
    if let Some(s) = q.start.as_deref().filter(|s| !s.is_empty()) {
        qs = qs.filter_gt("created_at", dt_ge(s)).map_err(ApiError::from)?;
    }
    if let Some(e) = q.end.as_deref().filter(|s| !s.is_empty()) {
        qs = qs.filter_lt("created_at", dt_le(e)).map_err(ApiError::from)?;
    }
    // 与登录记录同规则：非超管按数据权限看（部门 / 仅本人，条件是内联整数，无注入面）
    qs = datascope::apply_admin_scope(qs, scope);
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
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let deleted = crate::api::delete_by_ids(&state, "audit_log", &ids).await?;
    Ok(ok(json!({ "deleted": deleted })))
}

fn header_cells() -> Vec<String> {
    ["ID", "时间", "用户", "模块", "动作", "方法", "路径", "结果", "详情", "耗时(ms)", "IP"]
        .map(String::from)
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee_orm::Value;
    use chrono::NaiveDateTime;
    use crate::datascope::DataScope;

    fn dt(s: &str) -> Value {
        Value::from(NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap())
    }

    /// 与登录记录同规则：闭区间 = 端点挪 1 秒 + 开区间，端点走绑定参数。
    #[test]
    fn date_bounds_are_closed_intervals_with_bound_params() {
        let q = AuditListQuery {
            page: None,
            size: None,
            username: None,
            module: Some("dict".into()),
            status: None,
            start: Some("2026-01-02 03:04:05".into()),
            end: Some("2026-01-02 09:00:00".into()),
        };
        let qs = filtered(&q, &DataScope { all: true, ..Default::default() }).unwrap();
        let sql = qs.to_sql();
        assert!(sql.contains("module = ?"), "{sql}");
        assert!(sql.contains("created_at > ?"), "{sql}");
        assert!(sql.contains("created_at < ?"), "{sql}");
        assert!(!sql.contains("2026-01-02"), "端点必须走绑定参数: {sql}");
        assert_eq!(
            qs.params(),
            &[
                Value::from("dict"),
                dt("2026-01-02 03:04:04"), // start - 1s（`>= start`）
                dt("2026-01-02 09:00:01"), // end + 1s（`<= end`）
            ],
        );
    }
}

/// 取一批导出数据（keyset：`id < last`，首页不带该条件），行渲染与旧的一次性实现逐字段一致。
async fn export_batch(
    state: &AppState,
    q: &AuditListQuery,
    scope: &datascope::DataScope,
    last: Option<i64>,
) -> Result<Vec<(i64, String)>, ApiError> {
    // 上游 `QuerySet` 不是 `Clone`（导出闭包每批调一次，得能重建）：按同一套筛选条件
    // 现搭一个，SQL 与 list 逐字一致（`filtered()` 只拼条件串，不查库）。
    let mut qs = filtered(q, scope)?.order_by("id DESC").limit(csv::BATCH);
    if let Some(last) = last {
        qs = qs.filter_lt("id", last).map_err(ApiError::from)?;
    }
    let rows = qs.all(&state.db).await.map_err(ApiError::from)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.id,
                csv::row(&[
                    hid::enc(r.id),
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
    csv::streamed(format!("audit-logs-{}.csv", csv::today()), header_cells(), move |last| {
        let state = state.clone();
        let q = q.clone();
        let scope = scope.clone();
        async move { export_batch(&state, &q, &scope, last).await }
    })
    .await
}
