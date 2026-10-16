// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 批次 E S1：动态验证码**一枚码只能用一次**（同一步内重放被拒）。
//!
//! 语义（lead 定的口径）：验证通过时把命中的步号写进 `admin.totp_last_step`，
//! 之后只有 `步号 > last_step` 的码能过 —— 同一个 30 秒窗里的第二台设备（或
//! 截了码的人）必须等下一枚码。代价：同窗两台设备连续登录只有第一台成功。
//!
//! 测试侧的码仍然按 RFC 4226 独立算（不 import src），服务端记的步号直接查库断言。
mod common;

use hmac::{Hmac, Mac};
use reqwest::Method;
use serde_json::{Value, json};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// 设备指纹是 UA（本项目口径）：不同 UA = 不同设备行。
fn client(ua: &str) -> reqwest::Client {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(reqwest::header::USER_AGENT, ua.parse().unwrap());
    reqwest::Client::builder().default_headers(h).build().unwrap()
}

fn b32_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            '2'..='7' => c as u32 - '2' as u32 + 26,
            _ => continue,
        };
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// RFC 4226 HOTP（6 位），`counter = unix / 30`（RFC 6238，步长 30、SHA1）。
fn code_at(secret_b32: &str, unix: u64) -> String {
    let key = b32_decode(secret_b32);
    let mut mac = HmacSha1::new_from_slice(&key).expect("HMAC 任意长度密钥");
    mac.update(&(unix / 30).to_be_bytes());
    let d = mac.finalize().into_bytes();
    let off = (d[19] & 0x0f) as usize;
    let bin = u32::from_be_bytes([d[off] & 0x7f, d[off + 1], d[off + 2], d[off + 3]]);
    format!("{:06}", bin % 1_000_000)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// 码 + 它属于哪一步：服务端会把这步记进账本，断言得拿它比。
fn code_step(secret: &str, offset_secs: u64) -> (String, i64) {
    let t = now_unix() + offset_secs;
    (code_at(secret, t), (t / 30) as i64)
}

async fn call(
    c: &reqwest::Client,
    method: Method,
    url: String,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let mut rb = c.request(method, url);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    let r = rb.send().await.expect("请求发送失败");
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

async fn login(c: &reqwest::Client, base: &str, user: &str, pass: &str, code: Option<&str>) -> (u16, Value) {
    let mut body = json!({ "username": user, "password": pass });
    if let Some(code) = code {
        body["code"] = json!(code);
    }
    call(c, Method::POST, format!("{base}/api/v1/auth/login"), None, Some(body)).await
}

/// 库里的账本值（NULL = 没记过 = 下一枚码随便用）。
async fn ledger(pool: &sqlx::MySqlPool) -> Option<i64> {
    sqlx::query_scalar("SELECT totp_last_step FROM admin WHERE username = 'admin'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn totp_code_is_single_use_per_step() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let a = client("replay-dev-A");
    let b = client("replay-dev-B");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── 0. 绑定：登录（未绑不要码）→ setup → enable ──
    let (st, v) = login(&a, &base, "admin", "admin123", None).await;
    assert_eq!(st, 200, "未绑定时不该要验证码: {v}");
    let token = v["data"]["token"].as_str().unwrap().to_string();
    let (st, v) = call(&a, Method::POST, api("/auth/totp/setup"), Some(&token), None).await;
    assert_eq!(st, 200, "生成密钥: {v}");
    let secret = v["data"]["secret"].as_str().unwrap().to_string();

    // ── 1. 负控：未绑定时 code 字段根本不看（现状口径），账本也不该有值 ──
    let (st, v) = login(&b, &base, "admin", "admin123", Some("000000")).await;
    assert_eq!(st, 200, "没绑定就不校验验证码（带什么码都能登）: {v}");
    assert_eq!(ledger(&pool).await, None, "未绑定时登录不该写账本");

    // ── 2. 绑定用的那枚码不写登录账本（绑定不是登录）──
    let (c_bind, _) = code_step(&secret, 0);
    let (st, v) = call(
        &a,
        Method::POST,
        api("/auth/totp/enable"),
        Some(&token),
        Some(json!({ "code": c_bind })),
    )
    .await;
    assert_eq!(st, 200, "绑定: {v}");
    assert_eq!(ledger(&pool).await, None, "绑定成功后登录账本仍是空");

    // ── 3. 第一次登录：账本记下所用步号 ──
    let (c1, n1) = code_step(&secret, 0);
    let (st, v) = login(&b, &base, "admin", "admin123", Some(&c1)).await;
    assert_eq!(st, 200, "对码登录: {v}");
    let token_b = v["data"]["token"].as_str().unwrap().to_string();
    assert_eq!(ledger(&pool).await, Some(n1), "验证通过要把所用步号记账");
    // 步号是内部账本：任何接口（profile / 列表）都不该带出去
    let (st, v) = call(&b, Method::GET, api("/auth/profile"), Some(&token_b), None).await;
    assert_eq!(st, 200, "{v}");
    assert!(
        v["data"]["user"].get("totp_last_step").is_none(),
        "步号不该出现在接口响应里: {v}"
    );

    // ── 4. 同一枚码再用一次（另一台设备/截了码的人）→ 拒 ──
    let devs_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_login_device")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (st, v) = login(&a, &base, "admin", "admin123", Some(&c1)).await;
    assert_eq!(st, 400, "同码重放必须被拒（这是本条的全部意义）: {v}");
    assert_eq!(v["err"], "auth.totp_replayed", "{v}");
    assert!(v["data"]["token"].is_null(), "被拒的重放不能发 token: {v}");
    assert_eq!(ledger(&pool).await, Some(n1), "被拒的重放不改账本");
    let devs_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_login_device")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(devs_after, devs_before, "被拒的重放不落设备行");

    // ── 5. 错码也不写账本（这次账本已有值，断言「没被改」）──
    let (st, v) = login(&a, &base, "admin", "admin123", Some("000000")).await;
    assert_eq!(st, 400, "错码: {v}");
    assert_eq!(v["err"], "auth.bad_totp", "{v}");
    assert_eq!(ledger(&pool).await, Some(n1), "错码不该动账本");

    // ── 6. 下一窗的码照用（账本是**高水位**，不是「一枚码一次性」的指纹）──
    let (c2, n2) = code_step(&secret, 30);
    assert!(n2 > n1, "测试前提：下一步得真的更大（{n2} > {n1}）");
    let (st, v) = login(&a, &base, "admin", "admin123", Some(&c2)).await;
    assert_eq!(st, 200, "新一步的码必须能过（时钟走一步不该被账本挡住）: {v}");
    assert_eq!(ledger(&pool).await, Some(n2), "账本推进到新步");

    // 6b. 高水位之后，回头用**更早**的码也拒（不是只认「同一枚码」）
    let (st, v) = login(&a, &base, "admin", "admin123", Some(&c1)).await;
    assert_eq!(st, 400, "过期步的码（哪怕没重放过）也过不了: {v}");
    assert_eq!(v["err"], "auth.totp_replayed", "{v}");

    // ── 7. 解绑清账本（留着等于给重开的下次留了个高水位）──
    let (c3, _) = code_step(&secret, 30);
    let (st, v) = call(
        &a,
        Method::POST,
        api("/auth/totp/disable"),
        Some(&token_b),
        Some(json!({ "code": c3 })),
    )
    .await;
    assert_eq!(st, 200, "解绑: {v}");
    assert_eq!(ledger(&pool).await, None, "解绑要把账本清成 NULL（重新绑定从零开始）");
}
