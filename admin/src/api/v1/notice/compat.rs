// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! `/api/v1` 公告列表 / 未读的兼容实现：保持 v2.18.1 之前的完整行（含正文）。
//!
//! 摘要化是破坏性变更，只能进 `/api/v2`；v1 调用方不应该因为升级被迫改字段。
use super::{NoticeQuery, UNREAD_LIMIT, fetch_notice_page};
use crate::auth::Auth;
use crate::error::{ApiError, AppQuery, ok};
use crate::models::Notice;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use bee_orm::{Model, Value as DbValue};
use serde_json::{Value, json};

/// v1 契约：列表行是完整公告（含 `content` / `created_by`）。
#[apidoc::title("公告列表（v1 兼容）")]
#[apidoc::desc("v1 保持完整行（含正文）；新调用方请用 /api/v2/notices 的摘要列表")]
#[apidoc::url("/api/v1/notices")]
#[apidoc::method("GET")]
#[apidoc::tag("公告")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100")]
#[apidoc::query(name = "title", ty = "string", desc = "标题模糊匹配")]
#[apidoc::query(name = "status", ty = "int", desc = "状态：1 已发布 / 0 草稿")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "分页结果（行含正文）", children = [
            {name = "list", ty = "array", desc = "公告完整对象：id / title / content / status / created_by / published_at / created_at / updated_at"},
            {name = "total", ty = "int", desc = "总数"},
        ])]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<NoticeQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:notice:list")?;
    // 与 v2 摘要列表共用筛选/分页口径，只有 SELECT 的列不同（v1 保持完整行）。
    let (rows, total) = fetch_notice_page(&state, &q, "*").await?;
    let list =
        rows.iter().map(Notice::from_row).collect::<Result<Vec<_>, _>>().map_err(ApiError::from)?;
    Ok(ok(json!({ "list": list, "total": total })))
}

/// v1 契约：未读行同样是完整公告（含 `content`）。
#[apidoc::title("我的未读公告（v1 兼容）")]
#[apidoc::desc("v1 保持完整行（含正文）；新调用方请用 /api/v2/notices/unread 的摘要")]
#[apidoc::url("/api/v1/notices/unread")]
#[apidoc::method("GET")]
#[apidoc::tag("公告")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "未读结果（行含正文）", children = [
            {name = "list", ty = "array", desc = "最近 50 条未读公告完整对象"},
            {name = "total", ty = "int", desc = "未读总数（不受 50 条上限影响）"},
        ])]
pub async fn unread(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    // 「已发布 + 我没读过」是 LEFT JOIN 的语义，保留带参裸 SQL，值仍走绑定。
    let total: i64 = state
        .db
        .query(
            "SELECT COUNT(*) AS n FROM notice n \
             LEFT JOIN notice_read r ON r.notice_id = n.id AND r.admin_id = ? \
             WHERE n.status = 1 AND r.notice_id IS NULL",
            &[DbValue::from(auth.admin.id)],
        )
        .await
        .map_err(ApiError::from)?
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let rows = state
        .db
        .query(
            "SELECT n.* FROM notice n \
             LEFT JOIN notice_read r ON r.notice_id = n.id AND r.admin_id = ? \
             WHERE n.status = 1 AND r.notice_id IS NULL \
             ORDER BY n.id DESC LIMIT ?",
            &[DbValue::from(auth.admin.id), DbValue::from(UNREAD_LIMIT)],
        )
        .await
        .map_err(ApiError::from)?;
    let list =
        rows.iter().map(Notice::from_row).collect::<Result<Vec<_>, _>>().map_err(ApiError::from)?;
    Ok(ok(json!({ "total": total, "list": list })))
}
