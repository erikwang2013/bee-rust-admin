// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

use super::config::{DEFAULT_ENCRYPTION, DEFAULT_SMTP_PORT, truthy};
use super::mail::redact_addr;
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
    // 默认（两个渠道都关）随便存：没启用就不校验地址是否可发
    assert!(validate(&cfg()).is_ok());
    // 但长度**关着也拦**：超长值整条进审计摘要，写不进 changes 就丢一条审计
    let mut c = cfg();
    c.webhook_url = format!("https://x.com/{}", "a".repeat(600));
    assert!(validate(&c).is_err(), "关着也不许存超长地址");
    c.webhook_url = format!("https://x.com/{}", "a".repeat(460));
    assert_eq!(c.webhook_url.chars().count(), 474, "别把断言写超 512");
    assert!(validate(&c).is_ok(), "512 以内的地址关着可以存");

    // SMTP 的三个字符串字段同口径（与 webhook 同类）：关着也不许存超长值。
    // 这三个字段此前一处都没校验过，没有「沿用现有限值」可循 —— 取 255：
    // 主机名与邮箱地址的 RFC 上限都在这个量级以内。
    for (label, c) in [
        ("smtp_host", {
            let mut c = cfg();
            c.smtp_host = "h".repeat(300);
            c
        }),
        ("smtp_user", {
            let mut c = cfg();
            c.smtp_user = "u".repeat(300);
            c
        }),
        ("smtp_from", {
            let mut c = cfg();
            c.smtp_from = "f".repeat(300);
            c
        }),
    ] {
        assert!(validate(&c).is_err(), "SMTP 关着也不许存超长 {label}");
    }
    let mut c = cfg();
    c.smtp_host = "h".repeat(255);
    c.smtp_user = "u".repeat(255);
    c.smtp_from = "f".repeat(255);
    assert!(validate(&c).is_ok(), "255 以内的三个字段关着可以存（关闭时也不要求可发）");

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

/// A4：Webhook 的 **IP 字面量**不许指向链路本地（169.254/16、fe80::/10）或未指定
/// （0.0.0.0、::）—— 典型目标是云厂商元数据服务。回环 127.0.0.1 保持放行
/// （本机联调与既有验收依赖它）；域名解析到 link-local 不在本层拦（见函数注释）。
#[test]
fn webhook_rejects_link_local_and_unspecified_literals() {
    for bad in [
        "http://169.254.169.254/latest/meta-data",
        "http://[fe80::1]/hook",
        "http://0.0.0.0/hook",
        "http://[::]/hook",
        "http://[::ffff:169.254.169.254]/hook",
        // 解析差异绕过：reqwest（WHATWG）会把下面这些归一成 169.254.169.254
        r"http://169.254.169.254\@example.com/hook",
        "http://2852039166/",
        "http://0xa9fea9fe/",
        "http://0251.0376.0251.0376/",
        "http://169.254.169.254./",
        "http://169.254.\t169.254/",
    ] {
        let mut c = cfg();
        c.webhook_enabled = true;
        c.webhook_url = bad.into();
        match validate(&c) {
            Err(ApiError::BadRequest(msg)) => assert!(
                msg.contains("链路本地") && msg.contains("未指定"),
                "{bad} 的提示要说清拦的是什么: {msg}"
            ),
            other => panic!("{bad} 应 400，实际 {other:?}"),
        }
    }
    // IPv6 zone id 被 reqwest 的解析器直接拒（发不出去），提示与链路本地不同
    let mut c = cfg();
    c.webhook_enabled = true;
    c.webhook_url = "http://[fe80::1%25eth0]/hook".into();
    match validate(&c) {
        Err(ApiError::BadRequest(msg)) => assert!(msg.contains("无法解析"), "{msg}"),
        other => panic!("zone id 应 400，实际 {other:?}"),
    }
    for ok in [
        "http://127.0.0.1:9/hook",
        "http://192.168.1.2/hook",
        "https://example.com/hook",
        // 主机名不是 IP 字面量：本层不解析
        "http://localhost:9/hook",
    ] {
        let mut c = cfg();
        c.webhook_enabled = true;
        c.webhook_url = ok.into();
        assert!(validate(&c).is_ok(), "{ok} 应放行");
    }
}

/// M2：投递失败的原因里不能出现收件地址（日志/面板只留账号 id）。
/// 解析错误与 SMTP 拒绝响应都可能回显地址，统一在 `redact_addr` 这一道替换。
#[test]
fn mail_errors_never_echo_the_recipient_address() {
    let addr = "alice@example.com";
    let rejected = format!("550 5.1.1 <{addr}>: Recipient address rejected");
    let got = redact_addr(rejected, addr);
    assert!(!got.contains(addr), "拒绝响应里的地址必须替换掉: {got}");
    assert!(got.contains("<收件人>"), "替换成占位符，保留可读性: {got}");
    let parse = redact_addr(format!("mailbox parse error: {addr}"), addr);
    assert!(!parse.contains(addr), "解析错误同样不许带地址: {parse}");
    assert_eq!(redact_addr("502 bad gateway".into(), addr), "502 bad gateway", "别的错误原样保留");
    assert_eq!(redact_addr("no address".into(), ""), "no address", "空地址不替换");
}
