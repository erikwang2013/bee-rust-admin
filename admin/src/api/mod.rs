// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::error::ApiError;
use crate::state::AppState;

pub mod admin;
pub mod audit_log;
pub mod auth;
pub mod csv;
pub mod dept;
pub mod login_log;
pub mod menu;
pub mod role;

/// 按 id 分批删（ORM 无按条件批量删）；返回删除条数。
/// `table` 只传调用方写死的常量，id 来自数据库，不拼用户输入。
pub(crate) async fn delete_by_ids(
    state: &AppState,
    table: &str,
    ids: &[u64],
) -> Result<u64, ApiError> {
    let mut deleted = 0u64;
    for chunk in ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let sql = format!("DELETE FROM {table} WHERE id IN ({placeholders})");
        let mut q = sqlx::query(&sql);
        for id in chunk {
            q = q.bind(id);
        }
        q.execute(state.db.pool())
            .await
            .map_err(|e| ApiError::from(bee_orm::OrmError::from(e)))?;
        deleted += chunk.len() as u64;
    }
    Ok(deleted)
}

/// (1 起始页码, 每页条数)；size 上限 100。
pub(crate) fn page_size(page: Option<u32>, size: Option<u32>) -> (usize, usize) {
    (
        page.unwrap_or(1).max(1) as usize,
        size.unwrap_or(10).clamp(1, 100) as usize,
    )
}

/// 关联 id 去重排序。**所有 `set_relations` 调用前必须先过这里**：
/// bee_orm 的 `set_relations` 不去重，传重复 id 会命中复合主键冲突、
/// 整批回滚并返回 `DuplicateKey`（真库实测确认）。
pub(crate) fn dedup_ids(mut ids: Vec<u64>) -> Vec<u64> {
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 客户端文本字段长度校验：超长直接 400，别让 MySQL 报 1406 变成 500，
/// 也别在非严格模式下被静默截断（截断后的权限码再也匹配不上）。
pub(crate) fn check_len(field: &str, value: &str, max: usize) -> Result<(), ApiError> {
    if value.chars().count() > max {
        return Err(ApiError::BadRequest(format!(
            "{field} 长度不能超过 {max} 个字符"
        )));
    }
    Ok(())
}

/// 树形防环（menu/dept 共用）：从 `parent` 沿 `parent_of` 向上走到根，
/// 碰到 `id` 说明把 `id` 挂到自己的后代下会成环。`hops` 兜底数据异常造成的环。
pub(crate) fn would_cycle(
    parent_of: &std::collections::HashMap<u64, u64>,
    id: u64,
    parent: u64,
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
    parent_of: &std::collections::HashMap<u64, u64>,
    ids: &[u64],
) -> Vec<u64> {
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
        let map: HashMap<u64, u64> = [(1, 0), (2, 1), (3, 2), (5, 6), (6, 5)].into_iter().collect();
        let mut got = with_ancestors(&map, &[3]);
        got.sort();
        assert_eq!(got, vec![1, 2, 3]);
        let mut cyc = with_ancestors(&map, &[5]);
        cyc.sort();
        assert_eq!(cyc, vec![5, 6]);
        assert_eq!(with_ancestors(&map, &[]), Vec::<u64>::new());
    }

    #[test]
    fn would_cycle_detects_self_and_descendants() {
        // 1 → 2 → 3
        let map: HashMap<u64, u64> = [(1, 0), (2, 1), (3, 2)].into_iter().collect();
        assert!(would_cycle(&map, 1, 1), "挂到自己下");
        assert!(would_cycle(&map, 1, 3), "挂到自己的后代下");
        assert!(!would_cycle(&map, 3, 1), "挂到祖先下合法");
        assert!(!would_cycle(&map, 2, 0), "挂到根合法");
    }

    #[test]
    fn dedup_ids_sorts_and_dedups() {
        assert_eq!(dedup_ids(vec![3, 1, 3, 2, 1]), vec![1, 2, 3]);
        assert_eq!(dedup_ids(vec![]), Vec::<u64>::new());
    }
}
