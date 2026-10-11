// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 密码复杂度最小加固（P6）集成测试：**新设 / 新改**密码必须 ≥ 8 字符且至少两类字符。
//! 三处调用点走同一份规则（建管理员 / 重置密码 / 个人改密），这里钉两端：
//! 建管理员（`POST /admins`）与个人改密（`PUT /auth/password`）。
//! 登录路径不校验复杂度 —— 存量密码不受影响，这条**不在这里断言**（造不出 6 位账号）。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

/// 统一请求：返回 (状态码, JSON body)；断言消息带上 body，失败好定位。
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

/// 建管理员的请求体（部门 0 / 无角色，超管建人时够用）。
fn create_body(username: &str, password: &str) -> Value {
    json!({"username": username, "password": password, "nickname": username})
}

#[tokio::test]
async fn password_policy_requires_8_chars_and_two_classes() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");

    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录失败");

    // ── 1. 建管理员：7 位 → 400（长度不够）
    let (st, v) = call(
        &c, Method::POST, api("/admins"), Some(&admin), Some(create_body("pw_len", "short7x")),
    )
    .await;
    assert_eq!(st, 400, "7 位密码必须拒: {v}");
    assert_eq!(v["err"], "admin.weak_password", "长度不足要带稳定码: {v}");
    assert!(v["msg"].as_str().unwrap_or("").contains("至少 8 位"), "msg: {v}");

    // ── 2. 建管理员：8 位但只有一类 → 400（类数不够，msg 与长度不足是**同一条**）
    let (st, v) = call(
        &c, Method::POST, api("/admins"), Some(&admin), Some(create_body("pw_cls", "aaaaaaaa")),
    )
    .await;
    assert_eq!(st, 400, "8 位单类必须拒: {v}");
    assert_eq!(v["err"], "admin.weak_password", "类数不足也要带同一个码: {v}");
    assert!(v["msg"].as_str().unwrap_or("").contains("两类"), "msg: {v}");

    // ── 3. 建管理员：8 位两类 → 200
    let (st, v) = call(
        &c, Method::POST, api("/admins"), Some(&admin), Some(create_body("pw_ok", "Aa123456")),
    )
    .await;
    assert_eq!(st, 200, "8 位两类必须过: {v}");

    // ── 4. 新账号登录，拿自己的 token（个人改密只认自己）
    let tk = common::login(&base, "pw_ok", "Aa123456").await.expect("新账号必须能登录");

    // ── 5. 个人改密：8 位单类 → 400（同一个码，message 里叫「新密码」）
    let (st, v) = call(
        &c,
        Method::PUT,
        api("/auth/password"),
        Some(&tk),
        Some(json!({"old_password": "Aa123456", "new_password": "weakpass"})),
    )
    .await;
    assert_eq!(st, 400, "改密 8 位单类必须拒: {v}");
    assert_eq!(v["err"], "auth.weak_new_password", "改密要带新密码的码: {v}");

    // ── 6. 个人改密：8 位两类 → 200，且**新密码真的生效**（改密踢下线，重新登录）
    let (st, v) = call(
        &c,
        Method::PUT,
        api("/auth/password"),
        Some(&tk),
        Some(json!({"old_password": "Aa123456", "new_password": "weak1234"})),
    )
    .await;
    assert_eq!(st, 200, "8 位两类改密必须过: {v}");
    assert!(
        common::login(&base, "pw_ok", "weak1234").await.is_some(),
        "改密后必须能用新密码登录"
    );
}
