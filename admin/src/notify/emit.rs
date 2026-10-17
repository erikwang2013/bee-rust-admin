// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 事件分发：异步 Webhook + 公告邮件。

use super::*;

/// 发一个事件（**异步、不阻塞业务**）：webhook 与公告邮件各自开关、互不影响。
///
/// 用 `tokio::spawn` 而不是直接 await：公告发布不该等一个可能挂住的第三方。
/// 代价是「投递失败只进日志」—— 验收与排错看日志，不看接口返回值。
///
/// `actor` 是**触发事件的人**（公告发布者）：邮件推送要把本人排除掉，不给自己发信。
/// 与事件无关的场景（账号锁定）传 `None`。
pub fn emit(state: &AppState, event: &'static str, data: Value, actor: Option<i64>) {
    let state = state.clone();
    tokio::spawn(async move {
        let cfg = match load(&state.db).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("读通知配置失败，事件 {event} 不投递: {e}");
                return;
            }
        };
        if cfg.webhook_enabled && !cfg.webhook_url.is_empty() {
            let body =
                json!({ "event": event, "at": chrono::Utc::now().timestamp(), "data": data });
            if let Err(e) = post_webhook(&cfg.webhook_url, &body).await {
                tracing::warn!("Webhook 投递失败（{event} → {}）: {e}", cfg.webhook_url);
            }
        }
        // 公告邮件（E2-6）：只认「发布」这一个事件；开关关着就什么都不做（默认就是关的）
        if event == EVENT_NOTICE_PUBLISHED && cfg.email_notice_enabled {
            mail_notice_published(&state, &cfg, &data, actor).await;
        }
    });
}
