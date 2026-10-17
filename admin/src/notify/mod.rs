// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 通知渠道（扩展工程项 6）：出站 Webhook + SMTP 测试邮件。
//!
//! 配置存在 `sys_config` 的 `notify.*` 键上（项 1 的表），但**读写都收口在这里**：
//! SMTP 口令要走 [`crate::crypto`] 落库加密，通用配置页只认字符串（那个页面也
//! 因此看不到、改不动口令键，见 `api::config::is_secret_key`）。
//! 每次投递/发信**现读**配置：事件是低频的，一次 SELECT 换「改完即生效、不重启」。
//!
//! 出站三件事：事件推 webhook、测试邮件、**公告发布的邮件推送**（E2-6：开关
//! `notify.email_notice_enabled`，默认关；收件人 = 全部启用且填了邮箱的账号，
//! 发布者本人除外，逐个投递、best-effort）。

use crate::crypto;
use crate::error::ApiError;
use crate::models::{Admin, SysConfig};
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

mod config;
mod emit;
mod mail;
mod webhook;

#[cfg(test)]
mod tests;

pub use config::{NotifyConfig, load, save, validate};
pub use emit::emit;
pub use mail::send_test_mail;

use mail::mail_notice_published;
use webhook::post_webhook;

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
/// 公告发布时给「全体启用且填了邮箱的账号（发布者本人除外）」发邮件（E2-6）。
/// 默认 **0**：新装不会因为配了 SMTP 就突然开始群发。
pub const KEY_EMAIL_NOTICE_ENABLED: &str = "notify.email_notice_enabled";

/// 事件名（webhook 请求体里的 `event`）。
pub const EVENT_NOTICE_PUBLISHED: &str = "notice.published";
pub const EVENT_ACCOUNT_LOCKED: &str = "account.locked";
