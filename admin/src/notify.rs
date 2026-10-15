// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 通知渠道（扩展工程项 6）：出站 Webhook + SMTP 测试邮件。
//!
//! 配置存在 `sys_config` 的 `notify.*` 键上（项 1 的表），但**读写都收口在这里**：
//! SMTP 口令要走 [`crate::crypto`] 落库加密，通用配置页只认字符串（那个页面也
//! 因此看不到、改不动口令键，见 `api::config::is_secret_key`）。
//! 每次投递/发信**现读**配置：事件是低频的，一次 SELECT 换「改完即生效、不重启」。
//!
//! v1 只做「出站」两件事：事件推 webhook、测试邮件。公告发布的**邮件**推送按施工图
//! 放 v2（事件已经有了，加一个「事件 → 收件人」的表就能接上）。
use crate::crypto;
use crate::error::ApiError;
use crate::models::SysConfig;
use crate::state::AppState;
use crate::util::now;
use bee_orm::pool::mysql::Pool;
use bee_orm::{Model, OrmError};
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

// ── 配置键（种子与读写共用一份常量，改名只改这里）─────────────
pub const KEY_WEBHOOK_ENABLED: &str = "notify.webhook_enabled";
pub const KEY_WEBHOOK_URL: &str = "notify.webhook_url";
pub const KEY_SMTP_ENABLED: &str = "notify.smtp_enabled";
pub const KEY_SMTP_HOST: &str = "notify.smtp_host";
pub const KEY_SMTP_PORT: &str = "notify.smtp_port";
pub const KEY_SMTP_USER: &str = "notify.smtp_user";
pub const KEY_SMTP_PASSWORD: &str = "notify.smtp_password";
pub const KEY_SMTP_FROM: &str = "notify.smtp_from";
pub const KEY_SMTP_ENCRYPTION: &str = "notify.smtp_encryption";

/// SMTP 传输方式。`none` 是明文 —— 给本机 relay 与联调用（施工图验收就是本机假 SMTP），
/// 跨公网必须用另外两种。
pub const ENCRYPTIONS: [&str; 3] = ["starttls", "ssl", "none"];
const DEFAULT_ENCRYPTION: &str = "starttls";
/// 提交口默认 587（`starttls` 与公网的惯例口），配了以配置为准。
const DEFAULT_SMTP_PORT: u16 = 587;
/// 单封测试邮件的命令超时：SMTP 握手+发信超过它就是「挂住了」，别让面板一直转圈。
const SMTP_TIMEOUT_SECS: u64 = 10;

/// 事件名（webhook 请求体里的 `event`）。
pub const EVENT_NOTICE_PUBLISHED: &str = "notice.published";
pub const EVENT_ACCOUNT_LOCKED: &str = "account.locked";

/// 单次投递超时（施工图：「超时 + 重试 3 次」）。
const TIMEOUT_SECS: u64 = 5;
/// 首发 + 重试 3 次 = 最多 4 次请求。**不是** 3 次全量尝试：施工图写的是「重试 3 次」，
/// 首发那次不算重试。
const MAX_ATTEMPTS: usize = 4;
/// 重试间隔（定值）：事件会排队等在这条异步任务里，指数退避在「对端只是抖了一下」的
/// 场景下反而把最后一个事件拖秒级。
const RETRY_DELAY_MS: u64 = 500;

/// 通知配置在内存里的样子。读写都经 [`load`] / [`save`]，不要在别处拼 SQL。
#[derive(Debug, Clone, Default)]
pub struct NotifyConfig {
    pub webhook_enabled: bool,
    pub webhook_url: String,
    pub smtp_enabled: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_user: String,
    /// **库里的形态**（密文）。要用明文过 [`NotifyConfig::password`]，别直接读它。
    pub smtp_password: String,
    pub smtp_from: String,
    pub smtp_encryption: String,
}

impl NotifyConfig {
    /// 库里的口令 → 明文（存量明文原样返回，见 [`crate::crypto::plain`]）。
    pub fn password(&self, state: &AppState) -> Result<String, ApiError> {
        crypto::plain(&state.crypto, &self.smtp_password).map_err(ApiError::BadRequest)
    }

    /// 有没有设过口令（接口只回这个布尔，不回口令本身）。
    pub fn password_set(&self) -> bool {
        !self.smtp_password.is_empty()
    }
}

/// `"true"` / `"1"` 都算真（与 `api::config::public_value` 的布尔口径一致：
/// 库里被手写成 `"TRUE"` / `" true "` 时不至于被静默取反）。
fn truthy(v: &str) -> bool {
    let v = v.trim();
    v == "1" || v.eq_ignore_ascii_case("true")
}

fn bool_str(b: bool) -> String {
    if b { "true".into() } else { "false".into() }
}

/// 读全部 `notify.*`（一次 SELECT：这张表只有几十行，配置页也是这么读的）。
pub async fn load(db: &Pool) -> Result<NotifyConfig, OrmError> {
    let rows = SysConfig::query().all(db).await?;
    let map: HashMap<String, String> = rows.into_iter().map(|c| (c.key, c.value)).collect();
    let get = |k: &str| map.get(k).cloned().unwrap_or_default();
    Ok(NotifyConfig {
        webhook_enabled: truthy(&get(KEY_WEBHOOK_ENABLED)),
        webhook_url: get(KEY_WEBHOOK_URL).trim().to_string(),
        smtp_enabled: truthy(&get(KEY_SMTP_ENABLED)),
        smtp_host: get(KEY_SMTP_HOST).trim().to_string(),
        smtp_port: get(KEY_SMTP_PORT).trim().parse().unwrap_or(DEFAULT_SMTP_PORT),
        smtp_user: get(KEY_SMTP_USER).trim().to_string(),
        smtp_password: get(KEY_SMTP_PASSWORD),
        smtp_from: get(KEY_SMTP_FROM).trim().to_string(),
        smtp_encryption: {
            let e = get(KEY_SMTP_ENCRYPTION).trim().to_ascii_lowercase();
            if e.is_empty() { DEFAULT_ENCRYPTION.into() } else { e }
        },
    })
}

/// 落库（幂等 upsert，缺行就补）。`smtp_password` 传**要存进库的形态**（密文；空 = 置空）
/// ——「留空不改」是接口层的语义，在调用它之前就决掉。
pub async fn save(state: &AppState, cfg: &NotifyConfig) -> Result<(), ApiError> {
    let pairs: [(&str, String); 9] = [
        (KEY_WEBHOOK_ENABLED, bool_str(cfg.webhook_enabled)),
        (KEY_WEBHOOK_URL, cfg.webhook_url.clone()),
        (KEY_SMTP_ENABLED, bool_str(cfg.smtp_enabled)),
        (KEY_SMTP_HOST, cfg.smtp_host.clone()),
        (KEY_SMTP_PORT, cfg.smtp_port.to_string()),
        (KEY_SMTP_USER, cfg.smtp_user.clone()),
        (KEY_SMTP_PASSWORD, cfg.smtp_password.clone()),
        (KEY_SMTP_FROM, cfg.smtp_from.clone()),
        (KEY_SMTP_ENCRYPTION, cfg.smtp_encryption.clone()),
    ];
    for (key, value) in pairs {
        let row = SysConfig::query()
            .filter_eq("config_key", key)
            .map_err(ApiError::from)?
            .one(&state.db)
            .await
            .map_err(ApiError::from)?;
        match row {
            Some(mut c) => {
                if c.value == value {
                    continue; // 值没变就不写：updated_at 留着，审计也好读
                }
                c.value = value;
                c.updated_at = now();
                c.update(&state.db).await.map_err(ApiError::from)?;
            }
            None => {
                let c = SysConfig {
                    id: state.next_id()?,
                    key: key.to_string(),
                    value,
                    value_type: "string".into(),
                    group: "notify".into(),
                    is_public: 0, // 内部地址与口令，绝不进免登录的 /configs/public
                    remark: "由「通知设置」页维护".into(),
                    created_at: now(),
                    updated_at: now(),
                };
                c.insert(&state.db).await.map_err(ApiError::from)?;
            }
        }
    }
    Ok(())
}

/// 存之前的校验：只拦「一定发不出去」的配置，别拦「配了但对方不认」的（那要看失败原因）。
pub fn validate(cfg: &NotifyConfig) -> Result<(), ApiError> {
    if cfg.webhook_enabled {
        if cfg.webhook_url.is_empty() {
            return Err(ApiError::BadRequest("启用 Webhook 就要填地址".into()));
        }
        if !cfg.webhook_url.starts_with("http://") && !cfg.webhook_url.starts_with("https://") {
            return Err(ApiError::BadRequest("Webhook 地址必须是 http:// 或 https://".into()));
        }
        if cfg.webhook_url.chars().count() > 512 {
            return Err(ApiError::BadRequest("Webhook 地址过长（≤ 512 字符）".into()));
        }
    }
    if !ENCRYPTIONS.contains(&cfg.smtp_encryption.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "SMTP 加密方式只能是 {} 之一",
            ENCRYPTIONS.join(" / ")
        )));
    }
    if cfg.smtp_enabled {
        if cfg.smtp_host.is_empty() {
            return Err(ApiError::BadRequest("启用 SMTP 就要填服务器地址".into()));
        }
        if cfg.smtp_port == 0 {
            return Err(ApiError::BadRequest("SMTP 端口要在 1~65535 之间".into()));
        }
        let from = if cfg.smtp_from.is_empty() { &cfg.smtp_user } else { &cfg.smtp_from };
        if from.is_empty() {
            return Err(ApiError::BadRequest("启用 SMTP 就要填发件人".into()));
        }
        from.parse::<Mailbox>()
            .map_err(|e| ApiError::BadRequest(format!("发件人地址不合法（{from}）: {e}")))?;
    }
    Ok(())
}

/// 发一个事件（**异步、不阻塞业务**）：webhook 关着或没配地址就什么都不做。
///
/// 用 `tokio::spawn` 而不是直接 await：公告发布不该等一个可能挂住的第三方。
/// 代价是「投递失败只进日志」—— 验收与排错看日志，不看接口返回值。
pub fn emit(state: &AppState, event: &'static str, data: Value) {
    let state = state.clone();
    tokio::spawn(async move {
        let cfg = match load(&state.db).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("读通知配置失败，事件 {event} 不投递: {e}");
                return;
            }
        };
        if !cfg.webhook_enabled || cfg.webhook_url.is_empty() {
            return;
        }
        let body = json!({ "event": event, "at": chrono::Utc::now().timestamp(), "data": data });
        if let Err(e) = post_webhook(&cfg.webhook_url, &body).await {
            tracing::warn!("Webhook 投递失败（{event} → {}）: {e}", cfg.webhook_url);
        }
    });
}

/// 投一个 JSON 体，失败重试（见 [`MAX_ATTEMPTS`]）。返回最后一次的原因，成功返回 `Ok`。
async fn post_webhook(url: &str, body: &Value) -> Result<(), String> {
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

/// 用**已保存**的配置发一封测试邮件；失败把原因（lettre 的话）原样带回 ——
/// 面板上要能看出「为什么发不出去」。原因里不含口令：lettre 不回显凭据，
/// 这里也不拼任何配置值（发件人地址都不拼，那也算配置内容）。
pub async fn send_test_mail(cfg: &NotifyConfig, to: &str, password: &str) -> Result<(), String> {
    let from = if cfg.smtp_from.is_empty() { &cfg.smtp_user } else { &cfg.smtp_from };
    let email = Message::builder()
        .from(from.parse::<Mailbox>().map_err(|e| format!("发件人地址不合法（{from}）: {e}"))?)
        .to(to
            .parse::<Mailbox>()
            .map_err(|e| format!("收件人地址不合法（{to}）: {e}"))?)
        .subject("bee-rust-admin 邮件通知测试")
        .body(String::from(
            "这是一封来自 bee-rust-admin「通知设置」的测试邮件。收到即说明 SMTP 配置可用。",
        ))
        .map_err(|e| format!("组装邮件失败: {e}"))?;

    let host = cfg.smtp_host.as_str();
    let mut builder = match cfg.smtp_encryption.as_str() {
        // 465 隐式 TLS
        "ssl" => AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .map_err(|e| format!("初始化 TLS 失败: {e}"))?,
        // 明文。给本机 relay / 联调；跨公网别用
        "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
        // 587 STARTTLS：先明文连、再升级，升级失败就中止（不发凭据、不发信）
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| format!("初始化 TLS 失败: {e}"))?,
    };
    builder = builder
        .port(cfg.smtp_port) // 覆盖 relay/starttls_relay 挑的默认口：以配置为准
        .timeout(Some(Duration::from_secs(SMTP_TIMEOUT_SECS)));
    if !cfg.smtp_user.is_empty() {
        builder = builder.credentials(Credentials::new(cfg.smtp_user.clone(), password.to_string()));
    }
    // 忽略成功响应的 SMTP 回话文本：面板只需要「发出去了 / 为什么没发出去」
    builder.build().send(email).await.map(|_| ()).map_err(|e| format!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> NotifyConfig {
        NotifyConfig {
            smtp_encryption: DEFAULT_ENCRYPTION.into(),
            smtp_port: DEFAULT_SMTP_PORT,
            ..Default::default()
        }
    }

    #[test]
    fn truthy_accepts_one_and_true_case_insensitive() {
        for v in ["true", "TRUE", " true ", "1"] {
            assert!(truthy(v), "{v} 应为真");
        }
        for v in ["false", "0", "", "yes", "2"] {
            assert!(!truthy(v), "{v} 应为假");
        }
    }

    #[test]
    fn validate_rejects_only_unsendable_config() {
        // 默认（两个渠道都关）随便存：没启用就不校验地址
        assert!(validate(&cfg()).is_ok());

        let mut c = cfg();
        c.webhook_enabled = true;
        assert!(validate(&c).is_err(), "启用 webhook 却没地址");
        c.webhook_url = "ftp://x".into();
        assert!(validate(&c).is_err(), "非 http(s)");
        c.webhook_url = "http://127.0.0.1:1/hook".into();
        assert!(validate(&c).is_ok());

        let mut c = cfg();
        c.smtp_enabled = true;
        assert!(validate(&c).is_err(), "启用 smtp 却没服务器");
        c.smtp_host = "127.0.0.1".into();
        assert!(validate(&c).is_err(), "没发件人");
        c.smtp_from = "不是邮箱".into();
        assert!(validate(&c).is_err(), "发件人不合法");
        c.smtp_from = "a@b.c".into();
        assert!(validate(&c).is_ok());
        c.smtp_encryption = "tls".into();
        assert!(validate(&c).is_err(), "加密方式只认三种");
    }

}
