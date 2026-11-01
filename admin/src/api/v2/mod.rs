// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! v2 业务模块入口（对应 HTTP 面 `/api/v2`）。
//!
//! 只有发生破坏性变更的模块才在这里建文件；没变的 v1 契约继续用 `/api/v1`。
//! 路由仍由 `main.rs` 显式注册，模块不自己拼 URL。

pub mod notice;
