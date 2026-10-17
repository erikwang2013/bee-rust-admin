// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 配置模型、读写、校验与 Webhook 主机安全线。

use super::*;

/// SMTP 传输方式。`none` 是明文 —— 给本机 relay 与联调用（施工图验收就是本机假 SMTP），
/// 跨公网必须用另外两种。
pub const ENCRYPTIONS: [&str; 3] = ["starttls", "ssl", "none"];
pub(super) const DEFAULT_ENCRYPTION: &str = "starttls";
/// 提交口默认 587（`starttls` 与公网的惯例口），配了以配置为准。
pub(super) const DEFAULT_SMTP_PORT: u16 = 587;

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
    /// 公告发布时给全体收件人发邮件（§E2-6）。**必须**出现在 `api::notify::to_json` 里：
    /// 前端拿 GET 的回包回填整个表单再整体保存，少一个字段 = 保存一次就把它静默重置。
    pub email_notice_enabled: bool,
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
pub(super) fn truthy(v: &str) -> bool {
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
        email_notice_enabled: truthy(&get(KEY_EMAIL_NOTICE_ENABLED)),
    })
}

/// 落库（幂等 upsert，缺行就补）。`smtp_password` 传**要存进库的形态**（密文；空 = 置空）
/// ——「留空不改」是接口层的语义，在调用它之前就决掉。
pub async fn save(state: &AppState, cfg: &NotifyConfig) -> Result<(), ApiError> {
    let pairs: [(&str, String); 10] = [
        (KEY_WEBHOOK_ENABLED, bool_str(cfg.webhook_enabled)),
        (KEY_WEBHOOK_URL, cfg.webhook_url.clone()),
        (KEY_SMTP_ENABLED, bool_str(cfg.smtp_enabled)),
        (KEY_SMTP_HOST, cfg.smtp_host.clone()),
        (KEY_SMTP_PORT, cfg.smtp_port.to_string()),
        (KEY_SMTP_USER, cfg.smtp_user.clone()),
        (KEY_SMTP_PASSWORD, cfg.smtp_password.clone()),
        (KEY_SMTP_FROM, cfg.smtp_from.clone()),
        (KEY_SMTP_ENCRYPTION, cfg.smtp_encryption.clone()),
        (KEY_EMAIL_NOTICE_ENABLED, bool_str(cfg.email_notice_enabled)),
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

/// Webhook 出站的**最小** SSRF 防线：IP 字面量不许是链路本地（169.254/16、fe80::/10）
/// 或未指定（0.0.0.0、::）—— 典型目标是云厂商元数据服务（169.254.169.254）。
/// 回环放行：本机联调与既有验收依赖 127.0.0.1。
///
/// 必须用 reqwest 自己的解析器（`Url::parse`）取 host：反斜杠按 `/`、十进制/十六进制/
/// 八进制 IPv4、尾点、内嵌 tab 的归一化都与真正发请求时一致，手写切分会被解析差异绕过。
///
/// shortcut: 不做 DNS 解析 —— 域名解析到 link-local 仍可绕过这一层；真正的出站白名单
/// 应由部署侧网络策略承担，这里只挡「最省事的误配/探测」。
fn reject_unsafe_webhook_host(url: &str) -> Result<(), ApiError> {
    // 解析不了的地址一定发不出去（reqwest 用的是同一个解析器），且 zone id 这类
    // 写法会在这里失败；直接拦，别留「校验放行、发送才炸」的口子。
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return Err(ApiError::BadRequest(
            "Webhook 地址无法解析：URL 主机/端口不合法（IPv6 不支持 zone id 这类写法）".into(),
        ));
    };
    let Some(host) = parsed.host_str() else { return Ok(()) };
    // IPv6 的 host_str 带方括号（[fe80::1]），去掉后按 IP 字面量判
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let Ok(ip) = host.parse::<std::net::IpAddr>() else {
        return Ok(()); // 不是 IP 字面量（域名）：本层不处理
    };
    use std::net::IpAddr;
    let bad = match ip {
        IpAddr::V4(v4) => v4.is_link_local() || v4.is_unspecified(),
        IpAddr::V6(v6) => {
            let link_local = (v6.segments()[0] & 0xffc0) == 0xfe80;
            // ::ffff:169.254.169.254 这类映射地址按内层 v4 判
            let mapped_bad = v6
                .to_ipv4_mapped()
                .is_some_and(|v4| v4.is_link_local() || v4.is_unspecified());
            link_local || v6.is_unspecified() || mapped_bad
        }
    };
    if bad {
        return Err(ApiError::BadRequest(
            "Webhook 地址不能指向链路本地或未指定地址（169.254.0.0/16、fe80::/10、0.0.0.0、::）".into(),
        ));
    }
    Ok(())
}

/// 存之前的校验：只拦「一定发不出去」的配置，别拦「配了但对方不认」的（那要看失败原因）。
pub fn validate(cfg: &NotifyConfig) -> Result<(), ApiError> {
    // 长度与开关无关：关着也拦。这些值整条进审计摘要（`changes`），存个超长值会让
    // 那行摘要写不进去 —— 少一条审计比少一条通知严重（负控：audit_diff 的摘要列）。
    if cfg.webhook_url.chars().count() > 512 {
        return Err(ApiError::BadRequest("Webhook 地址过长（≤ 512 字符）".into()));
    }
    // 同上，SMTP 的三个字符串字段也关着拦。255 = 主机名 / 邮箱地址的 RFC 上限量级
    // （这三个字段此前没有限值，这里取 255 而不是另拟一套）。
    for (label, v) in [
        ("SMTP 服务器", &cfg.smtp_host),
        ("SMTP 用户名", &cfg.smtp_user),
        ("发件人", &cfg.smtp_from),
    ] {
        if v.chars().count() > 255 {
            return Err(ApiError::BadRequest(format!("{label}过长（≤ 255 字符）")));
        }
    }
    // SSRF 最小防线与开关无关：值先存下来，之后任何一次开启都会真发出去 —— 拦在入口
    // 比拦在出站便宜。只对 http(s) 判（别的 scheme 在启用时另有拒绝）。
    if cfg.webhook_url.starts_with("http://") || cfg.webhook_url.starts_with("https://") {
        reject_unsafe_webhook_host(&cfg.webhook_url)?;
    }
    if cfg.webhook_enabled {
        if cfg.webhook_url.is_empty() {
            return Err(ApiError::BadRequest("启用 Webhook 就要填地址".into()));
        }
        if !cfg.webhook_url.starts_with("http://") && !cfg.webhook_url.starts_with("https://") {
            return Err(ApiError::BadRequest("Webhook 地址必须是 http:// 或 https://".into()));
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
