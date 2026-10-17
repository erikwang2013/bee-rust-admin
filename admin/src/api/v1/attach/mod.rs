// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 附件中心（项 5）：分片上传 / 列表 / 下载 / 多态关联 / 删除。
//!
//! 上传复用 `api::avatar` 那套 aetherupload 内核接线（同一份 `[app] upload_dir` 下的
//! 另一个分组 `attach`，见 [`crate::api::avatar::build_runtime`]），差别只在闸门与落库：
//! 附件上限 10MB（`AVATAR_MAX_BYTES` 的 20 倍），落 `attachment` 行、留原文件名与 sha256。
//!
//! **下载鉴权（本模块的核心口径）**：
//! 1. 必须登录（不给公开路由 —— 头像那条公开路是给 `<img>` 的，附件不给）；
//! 2. 绑定了**已发布**公告的附件 = 公开：公告本身对所有登录用户可见，配图必须跟着可见；
//! 3. 其余一律按上传人的数据权限卡（与列表同一个口径 `datascope::ensure_admin_in_scope`）。
//!
//! 删除比下载更严：**公开规则不适用**（配图是公告的一部分，删附件得看数据权限），
//! 且要求 `system:attach:remove`。
use crate::api::{PagingExt, page_size};
use crate::api::avatar::{problem_to_api_error, run_kernel};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppJson, AppMultipart, AppPath, AppQuery, ok};
use crate::hid;
use crate::models::{Admin, Attachment, Notice};
use crate::state::AppState;
use crate::util::now;
use aetherupload::json::{save_chunk_ok_to_json, upload_ok_to_json};
use aetherupload::{ChunkBody, FormData, UploadController};
use axum::Json;
use axum::extract::{Multipart, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bee_orm::{Model, Value as DbValue};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

mod upload;
mod manage;

pub use self::upload::*;
pub use self::manage::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_name_and_whitelist_are_plugin_safe() {
        // 分组名含下划线会让 saved_path 三段式解码错位（插件直接拒）
        assert!(!ATTACH_GROUP.contains('_'));
        assert!(ATTACH_GROUP.len() <= 20);
        // 白名单里不能有可执行扩展名（黑名单那道只在白名单为空时才生效）
        for ext in ATTACH_EXTS {
            assert!(!matches!(ext, "php" | "exe" | "sh" | "js" | "jar" | "html" | "svg"), "{ext}");
        }
        // 内容反查用的扩展名（同族）也要在名单里，否则末片的内容校验过不去
        for ext in ["png", "jpg", "txt", "zip"] {
            assert!(ATTACH_EXTS.contains(&ext), "内容反查可能得出 {ext}");
        }
        // 旧版 office（OLE2）判成 octet-stream → 反查 bin，故意不收
        assert!(!ATTACH_EXTS.contains(&"bin"));
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sanitize_name_strips_paths_and_controls() {
        assert_eq!(sanitize_name("logo.png", "attach_x_y.png"), "logo.png");
        // 路径分隔符 / 控制字符 / 引号都换成空格（防「../」与引号破坏响应头）
        assert_eq!(sanitize_name("../../etc/passwd", "attach_x_y.png"), ".. .. etc passwd");
        assert_eq!(sanitize_name("a\"b\nc.png", "attach_x_y.png"), "a b c.png");
        // 空 / 全清掉的 → 退回落盘文件名（别给一个空名字）
        assert_eq!(sanitize_name("   ", "attach_202610_deadbeef.png"), "deadbeef.png");
        assert_eq!(sanitize_name("中文名.pdf", "attach_x_y.pdf"), "中文名.pdf");
        // 限长到列宽
        assert_eq!(sanitize_name(&"x".repeat(300), "attach_x_y.png").chars().count(), NAME_MAX);
    }
}
