// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 导出类型注册表与目录约定。

use super::*;

type BatchFuture = Pin<Box<dyn Future<Output = Result<Vec<(i64, String)>, ApiError>> + Send>>;

/// 一种可导出的数据。新增 kind = 在 `KINDS` 里加一条 + 前端加一个 i18n 名字，
/// 不加权限码（复用该模块的 `:list`：看得到才导得出）。
pub struct Kind {
    pub code: &'static str,
    /// 发起与下载都要过的查看码（= 该模块列表页的 `:list`）
    pub perm: &'static str,
    /// 落盘/下载文件名前缀
    pub file_prefix: &'static str,
    /// CSV 表头（与同步导出共用同一函数）
    pub headers: fn() -> Vec<String>,
    /// 发起时校验 params：与取数用**同一个** Deserialize 结构，不出现两套口径
    pub check: fn(&JsonVal) -> Result<(), String>,
    /// 按 params 取一批（keyset：`last` 是上一批最后一行 id）
    pub batch: fn(AppState, JsonVal, DataScope, Option<i64>) -> BatchFuture,
}

pub const KINDS: &[Kind] = &[
    Kind {
        code: "audit-log",
        perm: "system:auditlog:list",
        file_prefix: "audit-logs",
        headers: audit_log::headers,
        check: |p| parse::<audit_log::AuditListQuery>(p),
        batch: |s, p, sc, last| {
            Box::pin(async move {
                let q: audit_log::AuditListQuery =
                    serde_json::from_value(p).map_err(|e| ApiError::BadRequest(format!("筛选参数不合法: {e}")))?;
                audit_log::export_batch(&s, &q, &sc, last).await
            })
        },
    },
    Kind {
        code: "login-log",
        perm: "system:loginlog:list",
        file_prefix: "login-logs",
        headers: login_log::headers,
        check: |p| parse::<login_log::LogListQuery>(p),
        batch: |s, p, sc, last| {
            Box::pin(async move {
                let q: login_log::LogListQuery =
                    serde_json::from_value(p).map_err(|e| ApiError::BadRequest(format!("筛选参数不合法: {e}")))?;
                login_log::export_batch(&s, &q, &sc, last).await
            })
        },
    },
];

/// 按 code 找注册项；库里残留的旧 kind（已经下线的类型）找不到 = 任务失败。
pub fn spec(code: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.code == code)
}

fn parse<T: serde::de::DeserializeOwned>(p: &JsonVal) -> Result<(), String> {
    serde_json::from_value::<T>(p.clone()).map(|_| ()).map_err(|e| e.to_string())
}

/// 导出文件目录：`{upload_dir}/exports`（不单开配置项——它跟着上传目录走就够）。
pub fn dir(state: &AppState) -> PathBuf {
    Path::new(&state.cfg.upload_dir).join("exports")
}
