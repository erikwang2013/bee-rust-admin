// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::error::ApiError;
use crate::state::AppState;
use bee_orm::pool::mysql::Pool;
use bee_orm::{Model, OrmError, QuerySet, Value};

pub mod admin;
pub mod audit_log;
pub mod auth;
pub mod avatar;
pub mod csv;
pub mod dept;
pub mod dict;
pub mod job;
pub mod login_log;
pub mod menu;
pub mod notice;
pub mod role;

/// 按 id 分批删；返回**实际**删除条数。
/// `table` 只传调用方写死的常量，id 来自数据库，值一律走绑定参数。
pub(crate) async fn delete_by_ids(
    state: &AppState,
    table: &str,
    ids: &[i64],
) -> Result<i64, ApiError> {
    let mut deleted = 0i64;
    for chunk in ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let sql = format!("DELETE FROM {table} WHERE id IN ({placeholders})");
        let params: Vec<Value> = chunk.iter().map(|id| Value::from(*id)).collect();
        deleted += state.db.execute(&sql, &params).await.map_err(ApiError::from)? as i64;
    }
    Ok(deleted)
}

/// 分页取数。上游 `QuerySet` 只有 `limit`/`offset`，没有合起来的分页方法
/// （本地那套有 `fetch_page`）；这里补上，语义与迁移前一致：
/// 同一套筛选条件先数总数、再按 `limit/offset` 取当页，返回 `(当页行, 总数)`。
///
/// 接收者是 `self` 而不是 `&self`：`count` 借 `&self` 不消费，之后 `limit/offset`
/// 消费掉它 —— 这样调用点仍是 `query.order_by(...).fetch_page(db, page, size)`，
/// 一个字都不用改。
pub(crate) trait PagingExt<T>: Sized {
    async fn fetch_page(
        self,
        db: &Pool,
        page: usize,
        size: usize,
    ) -> Result<(Vec<T>, i64), OrmError>;
}

impl<T: Model> PagingExt<T> for QuerySet<T> {
    async fn fetch_page(
        self,
        db: &Pool,
        page: usize,
        size: usize,
    ) -> Result<(Vec<T>, i64), OrmError> {
        let total = self.count(db).await?;
        let offset = page.saturating_sub(1).saturating_mul(size);
        let rows = self.limit(size).offset(offset).all(db).await?;
        Ok((rows, total))
    }
}

/// `&[i64]` → `Vec<Value>`：上游 `filter_in` 收 `&[Value]`（值仍走绑定参数）。
pub(crate) fn ids_to_values(ids: &[i64]) -> Vec<Value> {
    ids.iter().map(|i| Value::from(*i)).collect()
}

/// 闭区间日期的「减一秒」值：给 `filter_gt` 用，表达 `>= T`。
///
/// 上游 `QuerySet` 只有**开区间**（`filter_gt` / `filter_lt`），没有 `>=` / `<=`，
/// 而 `filter(条件串)` 又不收参数（防注入的取舍）。本项目的时间列全是 `DATETIME`
/// （秒精度，DDL 里没有小数位），所以
///     `col >= T`  ≡  `col > T - 1s`
///     `col <= T`  ≡  `col < T + 1s`
/// 语义完全等价，**参数仍是绑定的**，没有走字符串拼接。
/// 解析不出时间格式时原样绑字符串（MySQL 自行比较），不让筛选条件凭空消失。
pub(crate) fn dt_ge(s: &str) -> Value {
    match chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S") {
        Ok(t) => Value::from(t - chrono::TimeDelta::seconds(1)),
        Err(_) => Value::from(s.trim()),
    }
}

/// 见 [`dt_ge`]：给 `filter_lt` 用，表达 `<= T`。
pub(crate) fn dt_le(s: &str) -> Value {
    match chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S") {
        Ok(t) => Value::from(t + chrono::TimeDelta::seconds(1)),
        Err(_) => Value::from(s.trim()),
    }
}

/// (1 起始页码, 每页条数)；size 上限 100。
pub(crate) fn page_size(page: Option<u32>, size: Option<u32>) -> (usize, usize) {
    (
        page.unwrap_or(1).max(1) as usize,
        size.unwrap_or(10).clamp(1, 100) as usize,
    )
}

/// 关联 id 去重排序。**所有 `set_relations` 调用前必须先过这里**：
/// 连接表有复合主键，传重复 id 会命中冲突把整批插入顶掉（真库实测确认）。
pub(crate) fn dedup_ids(mut ids: Vec<i64>) -> Vec<i64> {
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 客户端文本字段长度校验：超长直接 400，别让 MySQL 报 1406 变成 500，
/// 也别在非严格模式下被静默截断（截断后的权限码再也匹配不上）。
/// `field` 传**字段键**（如 `remark`，不是中文标签）：回 `err=common.too_long`
/// + `args={field,max}`，前端按 `field.<键>` 查文案；`msg` 里由键拼出中文（见 `field_label`）。
pub(crate) fn check_len(field: &str, value: &str, max: usize) -> Result<(), ApiError> {
    if value.chars().count() > max {
        return Err(ApiError::too_long(field, max));
    }
    Ok(())
}

/// 树形防环（menu/dept 共用）：从 `parent` 沿 `parent_of` 向上走到根，
/// 碰到 `id` 说明把 `id` 挂到自己的后代下会成环。`hops` 兜底数据异常造成的环。
pub(crate) fn would_cycle(
    parent_of: &std::collections::HashMap<i64, i64>,
    id: i64,
    parent: i64,
) -> bool {
    let mut cur = parent;
    let mut hops = 0;
    while cur != 0 && hops <= parent_of.len() {
        if cur == id {
            return true;
        }
        cur = *parent_of.get(&cur).unwrap_or(&0);
        hops += 1;
    }
    false
}

/// 沿 `parent_of` 把 ids 的祖先补全（含 ids 自身）。菜单树只勾了子节点时，
/// 没有祖先节点就拼不出树、侧边栏为空（antd 半选的父节点不会提交）。
/// `hops` 兜底数据异常造成的环。
pub(crate) fn with_ancestors(
    parent_of: &std::collections::HashMap<i64, i64>,
    ids: &[i64],
) -> Vec<i64> {
    let mut out = ids.to_vec();
    for id in ids {
        let mut cur = *parent_of.get(id).unwrap_or(&0);
        let mut hops = 0;
        while cur != 0 && hops <= parent_of.len() {
            if !out.contains(&cur) {
                out.push(cur);
            }
            cur = *parent_of.get(&cur).unwrap_or(&0);
            hops += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn with_ancestors_adds_chain_and_is_cycle_safe() {
        // 1 → 2 → 3；5 ⇄ 6 成环
        let map: HashMap<i64, i64> = [(1, 0), (2, 1), (3, 2), (5, 6), (6, 5)].into_iter().collect();
        let mut got = with_ancestors(&map, &[3]);
        got.sort();
        assert_eq!(got, vec![1, 2, 3]);
        let mut cyc = with_ancestors(&map, &[5]);
        cyc.sort();
        assert_eq!(cyc, vec![5, 6]);
        assert_eq!(with_ancestors(&map, &[]), Vec::<i64>::new());
    }

    #[test]
    fn would_cycle_detects_self_and_descendants() {
        // 1 → 2 → 3
        let map: HashMap<i64, i64> = [(1, 0), (2, 1), (3, 2)].into_iter().collect();
        assert!(would_cycle(&map, 1, 1), "挂到自己下");
        assert!(would_cycle(&map, 1, 3), "挂到自己的后代下");
        assert!(!would_cycle(&map, 3, 1), "挂到祖先下合法");
        assert!(!would_cycle(&map, 2, 0), "挂到根合法");
    }

    #[test]
    fn dedup_ids_sorts_and_dedups() {
        assert_eq!(dedup_ids(vec![3, 1, 3, 2, 1]), vec![1, 2, 3]);
        assert_eq!(dedup_ids(vec![]), Vec::<i64>::new());
    }
}
