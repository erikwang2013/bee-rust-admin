// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 公告列表的筛选 / 分页。v1 完整行与 v2 摘要共用这一份，只有 SELECT 的列不同。
use crate::api::page_size;
use crate::error::ApiError;
use crate::state::AppState;
use bee_orm::{Row, Value as DbValue};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct NoticeQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub title: Option<String>,
    pub status: Option<i8>,
}

/// 公告列表筛选的唯一口径（v1 完整行 / v2 摘要共用）：title 模糊 + status 等值。
/// 返回 `WHERE` 片段与按位绑定的参数；用户输入不进 SQL 文本。
pub(super) fn notice_filter(q: &NoticeQuery) -> (String, Vec<DbValue>) {
    let mut conds: Vec<&str> = Vec::new();
    let mut params: Vec<DbValue> = Vec::new();
    if let Some(t) = q.title.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        conds.push("title LIKE ?");
        params.push(format!("%{t}%").into());
    }
    if let Some(s) = q.status {
        conds.push("status = ?");
        params.push(s.into());
    }
    let where_sql =
        if conds.is_empty() { String::new() } else { format!(" WHERE {}", conds.join(" AND ")) };
    (where_sql, params)
}

/// 列表取数：筛选 / 排序 / 分页只有这一处实现，v1/v2 只差 SELECT 的列
/// （v1 完整行 `*`，v2 摘要列），所以语义不会漂。`columns` 只收调用方写死的常量。
pub(crate) async fn fetch_notice_page(
    state: &AppState,
    q: &NoticeQuery,
    columns: &str,
) -> Result<(Vec<Row>, i64), ApiError> {
    let (page, size) = page_size(q.page, q.size);
    let (where_sql, mut params) = notice_filter(q);
    let total = state
        .db
        .query(&format!("SELECT COUNT(*) AS n FROM notice{where_sql}"), &params)
        .await
        .map_err(ApiError::from)?
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    // 与 `PagingExt::fetch_page` 相同的偏移口径
    let offset = page.saturating_sub(1).saturating_mul(size);
    params.push(DbValue::from(size as i64));
    params.push(DbValue::from(offset as i64));
    let sql = format!("SELECT {columns} FROM notice{where_sql} ORDER BY id DESC LIMIT ? OFFSET ?");
    let rows = state.db.query(&sql, &params).await.map_err(ApiError::from)?;
    Ok((rows, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空白标题等于没筛；值仍走绑定参数。
    #[test]
    fn filter_keeps_title_and_status_semantics() {
        let q = NoticeQuery {
            page: None,
            size: None,
            title: Some("  维护 ".into()),
            status: Some(0),
        };
        let (sql, params) = notice_filter(&q);
        assert_eq!(sql, " WHERE title LIKE ? AND status = ?");
        assert_eq!(params, vec![DbValue::Text("%维护%".into()), DbValue::Int(0)]);

        let q = NoticeQuery { title: Some("   ".into()), status: None, ..q };
        let (sql, params) = notice_filter(&q);
        assert_eq!(sql, "", "空白标题不该变成 LIKE '%%'");
        assert!(params.is_empty());
    }
}
