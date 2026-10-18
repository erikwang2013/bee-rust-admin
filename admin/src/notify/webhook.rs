// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Webhook 投递：单次超时、HMAC 签名、发送前 DNS 封禁（F4-2 / F4-4）。
//!
//! 失败**不在本层重试**：首发失败落 `notify_delivery`，由 `notify_retry` 任务按
//! 1/5/15 分钟退避重投（F4-3）。

use super::config::is_blocked_ip;
use super::*;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::net::IpAddr;

/// 单次投递超时（施工图：「超时 + 重试 3 次」，重试已移到 `notify_retry`）。
const TIMEOUT_SECS: u64 = 5;
/// 发送前 DNS 解析的超时：解析卡住不能把事件任务一起挂死。
const RESOLVE_TIMEOUT_SECS: u64 = 5;

pub(super) const HEADER_TIMESTAMP: &str = "X-BRA-Timestamp";
pub(super) const HEADER_SIGNATURE: &str = "X-BRA-Signature";

/// 一次投递失败。`terminal` = 不许重投（发送时判定主机是链路本地/未指定，F4-4）。
pub(super) struct DeliveryError {
    pub(super) msg: String,
    pub(super) terminal: bool,
}

impl DeliveryError {
    fn retryable(msg: impl Into<String>) -> Self {
        Self { msg: msg.into(), terminal: false }
    }
    fn terminal(msg: impl Into<String>) -> Self {
        Self { msg: msg.into(), terminal: true }
    }
}

/// 发送前的主机判定结果。
enum Target {
    Safe,
    /// 链路本地/未指定：终态失败，不重投。
    Blocked(String),
    /// 解析失败/超时/URL 不可用：按普通投递失败处理（可重投）。
    Unresolved(String),
}

/// 解析结果里只要有任一地址命中封禁，就返回那一个（F4-4 的「任一结果」口径）。
/// 纯函数，单测直接喂解析结果。
fn blocked_addr(ips: impl IntoIterator<Item = IpAddr>) -> Option<IpAddr> {
    ips.into_iter().find(|ip| is_blocked_ip(*ip))
}

/// 发送前判定目标主机。IP 字面量直接判；域名用 tokio 解析（不再信任保存时的结论）。
///
/// shortcut: 解析与 reqwest 真正建连各自解析一次，理论上存在 DNS rebinding 的
/// TOCTOU 窗口；要彻底堵死得把连接钉在已校验的 IP 上，等有真实威胁场景再改。
async fn check_target(url: &str) -> Target {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return Target::Unresolved(format!("Webhook 地址无法解析: {url}"));
    };
    let Some(host) = parsed.host_str() else {
        return Target::Unresolved("Webhook 地址没有主机名".into());
    };
    // IPv6 的 host_str 带方括号（[fe80::1]），去掉后按 IP 字面量判
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_blocked_ip(ip) {
            Target::Blocked(format!("Webhook 主机 {ip} 是链路本地/未指定地址"))
        } else {
            Target::Safe
        };
    }

    let port = parsed.port_or_known_default().unwrap_or(80);
    match tokio::time::timeout(
        Duration::from_secs(RESOLVE_TIMEOUT_SECS),
        tokio::net::lookup_host((host, port)),
    )
    .await
    {
        Err(_) => Target::Unresolved(format!("解析 {host} 超时")),
        Ok(Err(e)) => Target::Unresolved(format!("解析 {host} 失败: {e}")),
        Ok(Ok(addrs)) => match blocked_addr(addrs.map(|a| a.ip())) {
            Some(ip) => Target::Blocked(format!("Webhook 域名 {host} 解析到链路本地/未指定地址 {ip}")),
            None => Target::Safe,
        },
    }
}

/// `sha256=hex(HMAC-SHA256(secret, ts + "." + body))`。签名对**实际发出的字节**算，
/// 所以 `body` 是序列化后的原文，不是再序列化一次。
fn sign(secret: &str, ts: u64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC 支持任意长度密钥");
    mac.update(ts.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!("sha256={}", crate::util::hex_lower(&mac.finalize().into_bytes()))
}

/// 投一个 JSON 体（**单次尝试**）。失败返回原因与是否终态。
pub(super) async fn post_webhook(url: &str, body: &Value, secret: &str) -> Result<(), DeliveryError> {
    match check_target(url).await {
        Target::Safe => {}
        Target::Blocked(msg) => return Err(DeliveryError::terminal(msg)),
        Target::Unresolved(msg) => return Err(DeliveryError::retryable(msg)),
    }

    let bytes = serde_json::to_vec(body)
        .map_err(|e| DeliveryError::retryable(format!("序列化事件体失败: {e}")))?;
    // 不跟随重定向（I1）：check_target 只校验初始 URL，跟随 302/307 会绕过发送时
    // 对链路本地地址的封禁。3xx 一律按普通投递失败处理（进重试）。
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| DeliveryError::retryable(format!("建 HTTP 客户端失败: {e}")))?;
    let mut req = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(bytes.clone());
    // 未配置密钥 = 不签（兼容旧接收方）；配了就签，时间戳与签名同一次生成
    if !secret.is_empty() {
        let ts = crate::util::now_unix();
        req = req
            .header(HEADER_TIMESTAMP, ts.to_string())
            .header(HEADER_SIGNATURE, sign(secret, ts, &bytes));
    }

    match req.send().await {
        Ok(r) if r.status().is_success() => Ok(()),
        Ok(r) => Err(DeliveryError::retryable(format!("HTTP {}", r.status().as_u16()))),
        Err(e) => Err(DeliveryError::retryable(format!("{e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 已知向量（独立用 python hmac/hashlib 算的固定值）：secret-key,
    /// ts = 1700000000, body = `{"event":"notice.published"}`。
    #[test]
    fn signature_is_hmac_sha256_of_ts_dot_body() {
        assert_eq!(
            sign("secret-key", 1_700_000_000, br#"{"event":"notice.published"}"#),
            "sha256=1e5c6eb66be401446bb836769daae2d22a1d2e73cfbfdff272fc7b7e2a159082"
        );
        assert_ne!(
            sign("other-key", 1_700_000_000, br#"{"event":"notice.published"}"#),
            sign("secret-key", 1_700_000_000, br#"{"event":"notice.published"}"#)
        );
    }

    /// F4-4：解析结果判定是纯函数；任一命中即拦（含 IPv4-mapped 与链路本地）。
    #[test]
    fn any_resolved_blocked_address_rejects_the_whole_target() {
        use std::net::Ipv4Addr;
        let ok = IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34));
        let meta = "169.254.169.254".parse::<IpAddr>().unwrap();
        let zero = "0.0.0.0".parse::<IpAddr>().unwrap();
        let v6_link = "fe80::1".parse::<IpAddr>().unwrap();
        let mapped = "::ffff:169.254.169.254".parse::<IpAddr>().unwrap();
        let loopback = "127.0.0.1".parse::<IpAddr>().unwrap();
        let local = "192.168.1.2".parse::<IpAddr>().unwrap();

        assert_eq!(blocked_addr(vec![ok]), None);
        assert_eq!(blocked_addr(vec![ok, local, loopback]), None);
        for bad in [meta, zero, v6_link, mapped] {
            assert_eq!(blocked_addr(vec![ok, bad]), Some(bad), "{bad} 必须拦");
        }
    }

    /// 字面量路径不发 DNS：链路本地直接终态，回环直接放行。
    #[tokio::test]
    async fn literal_hosts_are_checked_without_dns() {
        match check_target("http://169.254.169.254/latest/meta-data").await {
            Target::Blocked(msg) => assert!(msg.contains("链路本地"), "{msg}"),
            _ => panic!("链路本地字面量必须是终态拦截"),
        }
        assert!(matches!(check_target("http://127.0.0.1:9/hook").await, Target::Safe));
        assert!(matches!(check_target("http://[::]/hook").await, Target::Blocked(_)));
    }
}
