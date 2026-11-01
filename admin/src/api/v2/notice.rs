// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! `/api/v2/notices` 的 handler 入口。
//!
//! 列表/未读的摘要实现留在 `v1::notice`（v1 路由改用 `compat` 的完整行版本），
//! 其余公告接口 v1/v2 共用同一套逻辑，不复制。

pub use crate::api::v1::notice::{create, detail, list, read, remove, set_status, unread, update};
