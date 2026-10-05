// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
pub mod admin;
pub mod auth;
pub mod dept;
pub mod login_log;
pub mod menu;
pub mod role;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
