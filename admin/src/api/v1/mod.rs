// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! v1 业务模块入口（对应 HTTP 面 `/api/v1`）。
//!
//! 版本化约定：
//! - 业务模块一律是「目录 + `mod.rs`」：`v1/<module>/mod.rs` 是模块入口，需要拆分时
//!   子文件与 `mod.rs` 同级（`mod.rs` 里 `mod <子模块>;` + `pub use`），每个文件 <500 行；
//! - 共享设施（分页 / 长度校验 / 树工具 / CSV 等）留在 `api/`，不随版本复制；
//! - 将来 v2 按同样形状新增 `api/v2/`，由 `api/mod.rs` 显式挂载；v1 的旧引用路径保留；
//! - `api/mod.rs` 的 `pub use v1::*;` 只为保持既有 `crate::api::<module>` 路径
//!   （以及 `main.rs` 的路由注册）零改动。
//!
//! 本目录不含路由注册：URL 仍在 `main.rs` 里声明，模块搬家不改 URL / 契约 / 行为。

pub mod admin;
pub mod attach;
pub mod audit_log;
pub mod auth;
pub mod avatar;
pub mod config;
pub mod dashboard;
pub mod import;
pub mod dept;
pub mod export;
pub mod dict;
pub mod job;
pub mod login_log;
pub mod menu;
pub mod notice;
pub mod notify;
pub mod role;
pub mod totp;
