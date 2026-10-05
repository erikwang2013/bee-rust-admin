// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
pub mod admin;
pub mod auth;
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
