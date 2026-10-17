// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! SMTP 发信与公告邮件推送。

use super::*;

/// 单封测试邮件的命令超时：SMTP 握手+发信超过它就是「挂住了」，别让面板一直转圈。
const SMTP_TIMEOUT_SECS: u64 = 10;

/// 公告发布的邮件推送：收件人 = **全部启用且填了邮箱**的账号，**排除发布者本人**。
///
/// 逐个投递（一人一封、一人一次 SMTP 会话）：收件人之间互不抄送 —— 谁都能从
/// `To:` 里看到全体邮箱不合适，一个人失败也不该连累后面的人。**best-effort**：
/// 单封失败只记日志（不带收件地址），不重投、不回滚发布。
///
/// 开销是 **O(n) 次 SMTP 投递**（n = 符合条件的账号数）：公告发布本身不等它
/// （在 [`emit`] 的异步任务里跑），但发给一万人就是一万次会话 —— 大团队请先确认
/// SMTP 服务的发送配额（见 docs/usage.md §3.13）。
pub(super) async fn mail_notice_published(
    state: &AppState,
    cfg: &NotifyConfig,
    data: &Value,
    actor: Option<i64>,
) {
    // SMTP 没启用 = 用户没打算让它发信；别为每条公告刷一屏「连不上」的日志
    if !cfg.smtp_enabled || cfg.smtp_host.is_empty() {
        tracing::warn!("公告邮件推送开着但 SMTP 未启用/没配服务器，这次不发信");
        return;
    }
    let rows = match Admin::query().filter_eq("status", 1i8) {
        Ok(qs) => match qs.all(&state.db).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("公告邮件推送：查收件人失败: {e}");
                return;
            }
        },
        Err(e) => {
            tracing::warn!("公告邮件推送：查收件人失败: {e}");
            return;
        }
    };
    let password = match cfg.password(state) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("公告邮件推送：解 SMTP 口令失败: {e:?}");
            return;
        }
    };
    let title = data["title"].as_str().unwrap_or_default();
    let subject = format!("通知公告：{title}");
    let body = format!("公告《{title}》已发布，请登录 bee-rust-admin 后台查看全文。");
    let (mut sent, mut failed, mut skipped) = (0usize, 0usize, 0usize);
    for a in rows {
        if Some(a.id) == actor {
            continue; // 发布者本人不发
        }
        // email 是密文列（与个人中心同口径），空串 = 没填，解不开 = 这条跳过
        let to = match crypto::plain(&state.crypto, &a.email) {
            Ok(v) => v.trim().to_string(),
            Err(e) => {
                failed += 1;
                tracing::warn!("公告邮件推送：账号 {} 的邮箱解不开: {e}", a.id);
                continue;
            }
        };
        if to.is_empty() {
            skipped += 1;
            continue;
        }
        match send_mail(cfg, &to, &password, &subject, &body).await {
            Ok(()) => sent += 1,
            Err(e) => {
                failed += 1;
                // 不拼收件地址：邮箱是个人数据，日志里只留账号 id
                tracing::warn!("公告邮件推送：发给账号 {} 失败: {e}", a.id);
            }
        }
    }
    tracing::info!(
        "公告邮件推送（{}）: 发出 {sent} 封、失败 {failed}、没填邮箱 {skipped} 个",
        data["id"].as_str().unwrap_or("?")
    );
}

/// 用**已保存**的配置发一封测试邮件；失败把原因（lettre 的话）原样带回 ——
/// 面板上要能看出「为什么发不出去」。原因里不含口令：lettre 不回显凭据，
/// 这里也不拼任何配置值（发件人地址都不拼，那也算配置内容）。
pub async fn send_test_mail(cfg: &NotifyConfig, to: &str, password: &str) -> Result<(), String> {
    send_mail(
        cfg,
        to,
        password,
        "bee-rust-admin 邮件通知测试",
        "这是一封来自 bee-rust-admin「通知设置」的测试邮件。收到即说明 SMTP 配置可用。",
    )
    .await
}

/// 错误串里不带收件地址：邮箱是个人数据，日志/面板只留账号 id。lettre 的解析错误与
/// SMTP 拒绝响应都可能回显收件人（如「550 ... <a@b>」），拼进消息前替换掉。
/// shortcut: 只做原文替换（大小写变体的回显漏网）；地址来自同一个字符串，够用。
pub(super) fn redact_addr(msg: String, to: &str) -> String {
    if to.is_empty() {
        msg
    } else {
        msg.replace(to, "<收件人>")
    }
}

/// 发一封纯文本邮件：SMTP 连接/加密/认证这套公共部分（测试邮件与公告推送共用）。
/// 每次调用是一条独立的 SMTP 会话 —— 公告推送的「逐个投递」就是按这个粒度做的。
async fn send_mail(
    cfg: &NotifyConfig,
    to: &str,
    password: &str,
    subject: &str,
    text: &str,
) -> Result<(), String> {
    let from = if cfg.smtp_from.is_empty() { &cfg.smtp_user } else { &cfg.smtp_from };
    let email = Message::builder()
        .from(from.parse::<Mailbox>().map_err(|e| format!("发件人地址不合法（{from}）: {e}"))?)
        .to(to
            .parse::<Mailbox>()
            .map_err(|e| redact_addr(format!("收件人地址不合法: {e}"), to))?)
        .subject(subject)
        .body(String::from(text))
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
    // 忽略成功响应的 SMTP 回话文本：面板只需要「发出去了 / 为什么没发出去」。
    // 失败原因里若回显了收件人（SMTP 拒绝响应常见），替换掉再往外传。
    builder.build().send(email).await.map(|_| ()).map_err(|e| redact_addr(format!("{e}"), to))
}
