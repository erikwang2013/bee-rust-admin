// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 通知公告（v1.6.0 C2b）：管理侧增删改查 + 每个登录用户的未读/标记已读。
//! `unread` / `{id}/read` 只认登录（和字典下拉同理：每个登录用户都要有的能力，不挂权限码）。
use crate::api::{PagingExt, check_len, page_size};
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::hid;
use crate::models::Notice;
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use bee_orm::{Model, OrmError, Value as DbValue};
use chrono::NaiveDateTime;
use serde::Deserialize;
use serde_json::{Value, json};

/// 未读列表一次最多回多少条（角标用 `total`，不是这个长度）。
const UNREAD_LIMIT: i64 = 50;

/// 公告正文上限：TEXT 列是 64KB 字节，按中文 3 字节算 20000 字仍在列内，
/// 超了直接 400，别让 MySQL 报 1406 变成 500。
const CONTENT_MAX: usize = 20_000;

/// 发布语义：置 1 时**只在还是 NULL 时**写 `published_at`；从 1 改回 0 不清空（留痕：曾发布过）。
fn publish_at(status: i8, current: Option<NaiveDateTime>) -> Option<NaiveDateTime> {
    if status == 1 && current.is_none() {
        Some(now())
    } else {
        current
    }
}

fn check_title(title: &str) -> Result<(), ApiError> {
    if title.trim().is_empty() {
        return Err(ApiError::BadRequest("公告标题不能为空".into()));
    }
    check_len("title", title.trim(), 128)
}

fn check_content(content: &str) -> Result<(), ApiError> {
    if content.trim().is_empty() {
        return Err(ApiError::BadRequest("公告内容不能为空".into()));
    }
    check_len("content", content, CONTENT_MAX)
}

// ── 管理侧 ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct NoticeQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub title: Option<String>,
    pub status: Option<i8>,
}

#[derive(Deserialize)]
pub struct NoticeCreate {
    pub title: String,
    pub content: String,
    /// 0 草稿 / 1 已发布
    pub status: i8,
}

#[derive(Deserialize)]
pub struct NoticeUpdate {
    pub title: String,
    pub content: String,
    pub status: i8,
}

#[apidoc::title("公告列表")]
#[apidoc::desc("标题模糊 + 状态筛选 + 分页（管理页）")]
#[apidoc::url("/api/v1/notices")]
#[apidoc::method("GET")]
#[apidoc::tag("公告")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100")]
#[apidoc::query(name = "title", ty = "string", desc = "标题模糊匹配")]
#[apidoc::query(name = "status", ty = "int", desc = "状态：1 已发布 / 0 草稿")]
#[apidoc::response_status("200")]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<NoticeQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:notice:list")?;
    let (page, size) = page_size(q.page, q.size);
    let mut qs = Notice::query();
    if let Some(t) = q.title.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("title", t.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    let (rows, total) = qs
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

#[apidoc::title("新建公告")]
#[apidoc::desc("标题 / 正文长度校验不通过回 400")]
#[apidoc::url("/api/v1/notices")]
#[apidoc::method("POST")]
#[apidoc::tag("公告")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "title", ty = "string", required, desc = "标题")]
#[apidoc::param(name = "content", ty = "string", required, desc = "正文")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 发布 / 0 草稿，默认 0")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<NoticeCreate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:notice:add")?;
    check_title(&body.title)?;
    check_content(&body.content)?;

    let n = Notice {
        id: state.next_id()?,
        title: body.title.trim().to_string(),
        content: body.content,
        status: body.status,
        created_by: auth.admin.id,
        published_at: publish_at(body.status, None),
        created_at: now(),
        updated_at: now(),
    };
    // id 已在上面发号，insert 就够（create 的读回是给自增主键用的）
    n.insert(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(json!({ "id": hid::enc(n.id) })))
}

#[apidoc::title("编辑公告")]
#[apidoc::desc("发布时补 publish_at（草稿不补）")]
#[apidoc::url("/api/v1/notices/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("公告")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "公告对外 id（hashids 短串）")]
#[apidoc::param(name = "title", ty = "string", required, desc = "标题")]
#[apidoc::param(name = "content", ty = "string", required, desc = "正文")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 发布 / 0 草稿")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<NoticeUpdate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:notice:edit")?;
    let id = hid::dec(&id)?;
    check_title(&body.title)?;
    check_content(&body.content)?;

    let mut n = Notice::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    n.title = body.title.trim().to_string();
    n.content = body.content;
    n.status = body.status;
    n.published_at = publish_at(body.status, n.published_at);
    n.updated_at = now();
    n.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

/// 删公告连带删它的已读记录，**同一事务**（与字典的级联删一致）：
/// 残留行会让「已读」在 id 复用（本项目 id 不复用）时串味，留着也是垃圾。
#[apidoc::title("删除公告")]
#[apidoc::desc("连带删它的已读记录，**同一事务**")]
#[apidoc::url("/api/v1/notices/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("公告")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "公告对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:notice:remove")?;
    let id = hid::dec(&id)?;
    Notice::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    // notice_read 是复合主键（notice_id, admin_id），按主键删不适用，走带参裸 SQL；
    // Db 层没有事务，事务开在 Pool::get() 拿到的连接上（两条删除必须在同一个连接上）。
    let mut conn = state.db.get().await.map_err(ApiError::from)?;
    conn.begin().await.map_err(ApiError::from)?;
    let result = async {
        conn.execute("DELETE FROM notice_read WHERE notice_id = ?", &[DbValue::from(id)])
            .await?;
        conn.execute("DELETE FROM notice WHERE id = ?", &[DbValue::from(id)])
            .await?;
        Ok::<(), OrmError>(())
    }
    .await;
    match result {
        Ok(()) => conn.commit().await.map_err(ApiError::from)?,
        Err(e) => {
            let _ = conn.rollback().await;
            return Err(e.into());
        }
    }
    Ok(ok(Value::Null))
}

// ── 登录用户 ────────────────────────────────────────────────

/// 当前用户的未读列表：已发布 + 自己没读过，按 id DESC 取最近 50 条。
/// `total` 是**未读总数**（铃铛角标用它，不能拿 `list.length`——超过 50 条就少报了）。
#[apidoc::title("我的未读公告")]
#[apidoc::desc("登录即可。已发布 + 自己没读过，按 id DESC 取最近 50 条；total 是**未读总数**（铃铛角标用它）")]
#[apidoc::url("/api/v1/notices/unread")]
#[apidoc::method("GET")]
#[apidoc::tag("公告")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "未读结果", children = [
            {name = "list", ty = "array", desc = "最近 50 条未读公告"},
            {name = "total", ty = "int", desc = "未读总数（不受 50 条上限影响）"},
        ])]
pub async fn unread(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    // 「已发布 + 我没读过」是 LEFT JOIN 的语义（复合主键的 notice_read 表，
    // 关联不到 ORM 的单列外键上），保留带参裸 SQL，值仍走绑定。
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
    let list = rows
        .iter()
        .map(Notice::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "total": total, "list": list })))
}

/// 标记已读：**幂等**（`INSERT IGNORE` 语义），重复标记仍 200。
/// 目标是草稿或不存在时 404——草稿不属于任何人可见的未读集合（也不该借它探到草稿存在）。
#[apidoc::title("标记公告已读")]
#[apidoc::desc("登录即可。**幂等**（INSERT IGNORE 语义），重复标记仍 200；目标不存在或是草稿回 404")]
#[apidoc::url("/api/v1/notices/{id}/read")]
#[apidoc::method("POST")]
#[apidoc::tag("公告")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "公告对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn read(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    let id = hid::dec(&id)?;
    let n = Notice::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if n.status != 1 {
        return Err(ApiError::NotFound);
    }

    // INSERT IGNORE 保留裸 SQL：幂等靠主键冲突被忽略，先查后插在并发下会撞唯一键变 500
    state
        .db
        .execute(
            "INSERT IGNORE INTO notice_read (notice_id, admin_id, read_at) VALUES (?, ?, ?)",
            &[DbValue::from(id), DbValue::from(auth.admin.id), DbValue::from(now())],
        )
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn published_at_only_set_on_first_publish() {
        let was = at("2026-01-01 08:00:00");
        // 首次发布：写
        assert!(publish_at(1, None).is_some());
        // 重复发布：保留旧值（不是 now()）
        assert_eq!(publish_at(1, Some(was)), Some(was));
        // 撤回：不清空（留痕：曾发布过）
        assert_eq!(publish_at(0, Some(was)), Some(was));
        // 草稿从未发布过：保持空
        assert_eq!(publish_at(0, None), None);
    }

    #[test]
    fn title_and_content_rejected_when_blank_or_oversize() {
        assert!(check_title("  ").is_err());
        assert!(check_title("维护通知").is_ok());
        assert!(check_title(&"x".repeat(129)).is_err());
        assert!(check_title(&"x".repeat(128)).is_ok());
        assert!(check_content(" ").is_err());
        assert!(check_content("今晚停机").is_ok());
        assert!(check_content(&"x".repeat(CONTENT_MAX + 1)).is_err());
    }
}
