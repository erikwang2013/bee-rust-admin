// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Webhook 投递（超时 + 重试）。

use super::*;

/// 单次投递超时（施工图：「超时 + 重试 3 次」）。
const TIMEOUT_SECS: u64 = 5;
/// 首发 + 重试 3 次 = 最多 4 次请求。**不是** 3 次全量尝试：施工图写的是「重试 3 次」，
/// 首发那次不算重试。
const MAX_ATTEMPTS: usize = 4;
/// 重试间隔（定值）：事件会排队等在这条异步任务里，指数退避在「对端只是抖了一下」的
/// 场景下反而把最后一个事件拖秒级。
const RETRY_DELAY_MS: u64 = 500;

/// 投一个 JSON 体，失败重试（见 [`MAX_ATTEMPTS`]）。返回最后一次的原因，成功返回 `Ok`。
pub(super) async fn post_webhook(url: &str, body: &Value) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("建 HTTP 客户端失败: {e}"))?;
    let mut last = String::from("未知原因");
    for attempt in 1..=MAX_ATTEMPTS {
        // 4xx 也算失败并重试：对端「现在不收」和「500」在下游看来一样要重投
        match client.post(url).json(body).send().await {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) => last = format!("HTTP {}", r.status().as_u16()),
            Err(e) => last = format!("{e}"),
        }
        if attempt < MAX_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
        }
    }
    Err(last)
}
