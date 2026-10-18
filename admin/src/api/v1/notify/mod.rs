// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 通知渠道（扩展工程项 6）的三个接口：读配置 / 存配置 / 发一封测试邮件。
//! 权限码：读 `system:notify:list`，写与发信 `system:notify:edit`。
//!
//! 口令的处理口径（三处一致，改一处要一起看）：
//! - 读：只回 `smtp_password_set` 布尔，**永不回口令**；
//! - 写：体里 `smtp_password` 缺省 = 不改，空串 = 清空，非空 = 换新（`crypto::write` 落库加密）；
//! - 审计：只记「已变更」（`Diff::masked`），明文与密文都不进摘要。
use crate::audit::{AuditDetail, Diff, masked_changed};
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, ok};
use crate::notify::{self, NotifyConfig};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};

fn to_json(cfg: &NotifyConfig) -> Value {
    json!({
        "webhook_enabled": cfg.webhook_enabled,
        "webhook_url": cfg.webhook_url,
        // 签名密钥本身永不出口（同口令口径），只回是否已设置
        "webhook_secret_set": cfg.secret_set(),
        "smtp_enabled": cfg.smtp_enabled,
        "smtp_host": cfg.smtp_host,
        "smtp_port": cfg.smtp_port,
        "smtp_user": cfg.smtp_user,
        "smtp_from": cfg.smtp_from,
        "smtp_encryption": cfg.smtp_encryption,
        // 这个字段**必须**回：前端拿 GET 的回包回填整个表单、再整体保存，
        // 少回一个 = 用户点一次保存就把它静默重置成 false（见 notify::NotifyConfig 的注释）
        "email_notice_enabled": cfg.email_notice_enabled,
        // 口令本身永不出口；前端拿这个布尔决定口令框的占位提示
        "smtp_password_set": cfg.password_set(),
    })
}

#[apidoc::title("通知渠道配置")]
#[apidoc::desc("Webhook 与 SMTP 的配置（存在 sys_config 的 notify.* 键上）。口令只回「是否已设置」")]
#[apidoc::url("/api/v1/notify/config")]
#[apidoc::method("GET")]
#[apidoc::tag("通知设置")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::response_status("403")]
pub async fn get_config(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:notify:list")?;
    let cfg = notify::load(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(to_json(&cfg)))
}

#[derive(Deserialize)]
pub struct NotifyBody {
    #[serde(default)]
    pub webhook_enabled: bool,
    #[serde(default)]
    pub webhook_url: String,
    /// 缺省 = 不改；空串 = 清空；非空 = 换新（与 `smtp_password` 同一语义）。
    #[serde(default)]
    pub webhook_secret: Option<String>,
    #[serde(default)]
    pub smtp_enabled: bool,
    #[serde(default)]
    pub smtp_host: String,
    #[serde(default = "default_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub smtp_user: String,
    /// 缺省 = 不改；空串 = 清空；非空 = 换新（见模块注释）。
    #[serde(default)]
    pub smtp_password: Option<String>,
    #[serde(default)]
    pub smtp_from: String,
    #[serde(default = "default_encryption")]
    pub smtp_encryption: String,
    /// 公告发布时给全体启用账号发邮件（默认关）
    #[serde(default)]
    pub email_notice_enabled: bool,
}

fn default_port() -> u16 {
    587
}

fn default_encryption() -> String {
    "starttls".into()
}

#[apidoc::title("保存通知渠道配置")]
#[apidoc::desc("整体保存；`smtp_password` 缺省 = 不改、空串 = 清空、非空 = 换新（AES-256-GCM 落库加密）")]
#[apidoc::url("/api/v1/notify/config")]
#[apidoc::method("PUT")]
#[apidoc::tag("通知设置")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "webhook_enabled", ty = "bool", desc = "是否启用 Webhook")]
#[apidoc::param(name = "webhook_url", ty = "string", desc = "Webhook 地址（http/https）")]
#[apidoc::param(name = "webhook_secret", ty = "string", desc = "HMAC 签名密钥（缺省 = 不改；空串 = 清空；非空 = 换新）")]
#[apidoc::param(name = "smtp_enabled", ty = "bool", desc = "是否启用 SMTP")]
#[apidoc::param(name = "smtp_host", ty = "string", desc = "SMTP 服务器")]
#[apidoc::param(name = "smtp_port", ty = "int", desc = "端口，默认 587")]
#[apidoc::param(name = "smtp_user", ty = "string", desc = "认证用户名（空 = 不认证）")]
#[apidoc::param(name = "smtp_password", ty = "string", desc = "认证口令（缺省 = 不改）")]
#[apidoc::param(name = "smtp_from", ty = "string", desc = "发件人地址（空 = 用用户名）")]
#[apidoc::param(name = "smtp_encryption", ty = "string", desc = "starttls / ssl / none，默认 starttls")]
#[apidoc::param(name = "email_notice_enabled", ty = "bool", desc = "公告发布时给全体启用账号发邮件（发布者本人除外），默认 false")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("403")]
pub async fn save_config(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<NotifyBody>,
) -> Result<(AuditDetail, Json<Value>), ApiError> {
    auth.require("system:notify:edit")?;

    let old = notify::load(&state.db).await.map_err(ApiError::from)?;
    // 口令的三种语义在这里决掉，`notify::save` 只认「要存进库的形态」
    let (smtp_password, pw_changed) = match body.smtp_password.as_deref() {
        None => (old.smtp_password.clone(), false),
        Some("") => (String::new(), !old.smtp_password.is_empty()),
        Some(v) => {
            // 口径：口令也走密文落库；它落在 `sys_config.value`（TEXT），拦下来的是
            // `crypto::write` 的密文上限（255 字符），不是任何一列的列宽。
            let cipher =
                crate::crypto::write(&state.crypto, v).map_err(ApiError::BadRequest)?;
            (cipher, masked_changed(&state.crypto, &old.smtp_password, v))
        }
    };
    // 签名密钥三态与口令完全一致（F4-2）
    let (webhook_secret, secret_changed) = match body.webhook_secret.as_deref() {
        None => (old.webhook_secret.clone(), false),
        Some("") => (String::new(), !old.webhook_secret.is_empty()),
        Some(v) => {
            let cipher =
                crate::crypto::write(&state.crypto, v).map_err(ApiError::BadRequest)?;
            (cipher, masked_changed(&state.crypto, &old.webhook_secret, v))
        }
    };
    let new = NotifyConfig {
        webhook_enabled: body.webhook_enabled,
        webhook_url: body.webhook_url.trim().to_string(),
        webhook_secret,
        smtp_enabled: body.smtp_enabled,
        smtp_host: body.smtp_host.trim().to_string(),
        smtp_port: body.smtp_port,
        smtp_user: body.smtp_user.trim().to_string(),
        smtp_password,
        smtp_from: body.smtp_from.trim().to_string(),
        smtp_encryption: body.smtp_encryption.trim().to_ascii_lowercase(),
        email_notice_enabled: body.email_notice_enabled,
    };
    notify::validate(&new)?;
    notify::save(&state, &new).await?;

    // 变更摘要（项 8）：口令只记「改过」—— 记明文等于抵消落库加密，
    // 记密文每次随机 nonce 都不同、没有可比性（与 email/phone 同一口径）。
    let changes = Diff::default()
        .field("webhook_enabled", old.webhook_enabled, new.webhook_enabled)
        .field("webhook_url", old.webhook_url.clone(), new.webhook_url.clone())
        .masked("webhook_secret", secret_changed)
        .field("smtp_enabled", old.smtp_enabled, new.smtp_enabled)
        .field("smtp_host", old.smtp_host.clone(), new.smtp_host.clone())
        .field("smtp_port", old.smtp_port, new.smtp_port)
        .field("smtp_user", old.smtp_user.clone(), new.smtp_user.clone())
        .field("smtp_from", old.smtp_from.clone(), new.smtp_from.clone())
        .field("smtp_encryption", old.smtp_encryption.clone(), new.smtp_encryption.clone())
        .field("email_notice_enabled", old.email_notice_enabled, new.email_notice_enabled)
        .masked("smtp_password", pw_changed)
        .done();
    Ok((changes, ok(Value::Null)))
}

#[apidoc::title("发送测试邮件")]
#[apidoc::desc("用**已保存**的配置给当前登录者的邮箱发一封测试邮件；失败回 400 + 原因（不含口令）")]
#[apidoc::url("/api/v1/notify/test-email")]
#[apidoc::method("POST")]
#[apidoc::tag("通知设置")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("403")]
#[apidoc::returned(name = "data", ty = "object", desc = "{ \"to\": \"收件地址\" }")]
pub async fn test_email(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:notify:edit")?;
    let cfg = notify::load(&state.db).await.map_err(ApiError::from)?;
    if cfg.smtp_host.is_empty() {
        return Err(ApiError::BadRequest("先填 SMTP 服务器再发测试邮件".into()));
    }
    // 收件人 = 当前登录者自己的邮箱（admin.email 是密文列，发信前解密）。
    // 刻意**不**做成「随便填个收件人」：那等于把后台变成发信机。
    let to = crate::crypto::plain(&state.crypto, &auth.admin.email).map_err(ApiError::BadRequest)?;
    if to.trim().is_empty() {
        return Err(ApiError::BadRequest("你的账号还没填邮箱，先到「个人中心」补上".into()));
    }
    let password = cfg.password(&state)?;
    if let Err(e) = notify::send_test_mail(&cfg, to.trim(), &password).await {
        return Err(ApiError::BadRequest(format!("测试邮件发送失败: {e}")));
    }
    Ok(ok(json!({ "to": to.trim() })))
}
