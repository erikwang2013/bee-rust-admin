// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Webhook 失败落库与重投任务（F4-3）。
//!
//! 语义：首发失败（非 2xx / 超时 / 连接错 / DNS 解析失败）落 `pending`、attempts=1、
//! next_at=+1 分钟；`notify_retry` 每 5 分钟取 due 行重投，失败按 5/15 分钟退避，
//! 最多 3 次重投（attempts 到 4）后置 `failed`。发送时判定链路本地/未指定 = **终态
//! failed、不重投**（F4-4）。成功不落行（只有失败与终态进表）。30 天清理旧行。

use super::webhook::{DeliveryError, post_webhook};
use super::*;
use crate::models::NotifyDelivery;
use bee_orm::Model;

/// 首发 + 3 次重投。attempts 达到它（4）后不再重试。
const MAX_ATTEMPTS: i32 = 4;
/// 一轮最多处理多少条：单条最长 5s 超时，100 条已经远超一个 5 分钟的调度周期。
const BATCH: usize = 100;
/// 账本保留天数（成功/失败/终态一视同仁，按 created_at 清）。
const KEEP_DAYS: i64 = 30;

fn truncate(mut msg: String) -> String {
    msg.truncate(msg.char_indices().nth(255).map(|(i, _)| i).unwrap_or(msg.len()));
    msg
}

/// 首发失败落库：普通失败进 pending（+1 分钟重投），终态直接 failed。
pub(super) async fn record_failure(
    state: &AppState,
    event: &str,
    body: &Value,
    err: DeliveryError,
) {
    let id = match state.next_id() {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!("Webhook 失败账本发号失败，跳过落库: {e:?}");
            return;
        }
    };
    let created = now();
    let row = NotifyDelivery {
        id,
        event: event.to_string(),
        payload: body.to_string(),
        status: if err.terminal { "failed" } else { "pending" }.into(),
        attempts: 1,
        next_at: if err.terminal { None } else { Some(created + chrono::Duration::minutes(1)) },
        msg: truncate(err.msg),
        created_at: created,
    };
    if let Err(e) = row.insert(&state.db).await {
        tracing::warn!("Webhook 失败账本写库失败: {e}");
    }
}

/// 一次失败之后的退避（分钟）。首发失败由 `record_failure` 固定 +1；
/// 这里只处理重投失败：attempts=2 → 5 分钟，attempts=3 → 15 分钟，到 4 即终态。
fn next_delay_minutes(attempts: i32) -> i64 {
    match attempts {
        2 => 5,
        _ => 15,
    }
}

/// `notify_retry` 任务体：重投 due 行 + 30 天清理。返回的摘要进 `job_log.msg`。
pub async fn notify_retry(state: AppState) -> Result<String, String> {
    let cfg = load(&state.db).await.map_err(|e| format!("读通知配置失败: {e}"))?;
    let (mut tried, mut ok, mut failed, mut left) = (0u64, 0u64, 0u64, 0u64);

    if cfg.webhook_enabled && !cfg.webhook_url.is_empty() {
        let secret = cfg.secret(&state).map_err(|e| format!("签名密钥解密失败: {e:?}"))?;
        let rows = state
            .db
            .query(
                "SELECT * FROM notify_delivery \
                 WHERE status = 'pending' AND next_at IS NOT NULL AND next_at <= ? \
                 ORDER BY next_at ASC, id ASC LIMIT ?",
                &[bee_orm::Value::from(now()), bee_orm::Value::from(BATCH as i64)],
            )
            .await
            .map_err(|e| format!("查待重投记录失败: {e}"))?;

        for row in &rows {
            let mut d = NotifyDelivery::from_row(row)
                .map_err(|e| format!("读待重投记录失败: {e}"))?;
            tried += 1;
            // payload 是首发时实际发出的原文；重投原样再发（签名对同一份字节成立）
            let outcome = match serde_json::from_str::<Value>(&d.payload) {
                Ok(body) => post_webhook(&cfg.webhook_url, &body, &secret).await,
                Err(e) => Err(DeliveryError {
                    msg: format!("payload 非法: {e}"),
                    terminal: true,
                }),
            };
            d.attempts += 1;
            match outcome {
                Ok(()) => {
                    d.status = "delivered".into();
                    d.next_at = None;
                    d.msg = "重投成功".into();
                    ok += 1;
                }
                Err(e) => {
                    d.msg = truncate(e.msg);
                    if e.terminal || d.attempts >= MAX_ATTEMPTS {
                        d.status = "failed".into();
                        d.next_at = None;
                        failed += 1;
                    } else {
                        d.next_at =
                            Some(now() + chrono::Duration::minutes(next_delay_minutes(d.attempts)));
                        left += 1;
                    }
                }
            }
            d.update(&state.db)
                .await
                .map_err(|e| format!("回写待重投记录失败: {e}"))?;
        }
    }

    // 30 天清理：无论渠道开关都跑，账本不能无限攒
    let purged = state
        .db
        .execute(
            "DELETE FROM notify_delivery WHERE created_at < ?",
            &[bee_orm::Value::from(now() - chrono::Duration::days(KEEP_DAYS))],
        )
        .await
        .map_err(|e| format!("清理过期投递记录失败: {e}"))?;

    if cfg.webhook_enabled && !cfg.webhook_url.is_empty() {
        Ok(format!(
            "重投 {tried} 条：成功 {ok} / 失败 {failed} / 待重试 {left}；清理过期记录 {purged} 条"
        ))
    } else {
        Ok(format!("Webhook 未启用，跳过重投；清理过期记录 {purged} 条"))
    }
}
