// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 修复 1（匿名日志写放大）：审计 / 安全扫描 / 登录失败三条路径按来源 IP 限额，
//! 已鉴权审计与成功登录不受影响。额度是每窗口 20 条（见 `src/log_gate.rs`）。
mod common;

use reqwest::Method;
use serde_json::{Value, json};
use sqlx::MySqlPool;

const SQLI_QUERY: &str = "1%20UNION%20SELECT%20password%20FROM%20admin";

async fn call(
    c: &reqwest::Client,
    method: Method,
    url: String,
    token: Option<&str>,
    ip: Option<&str>,
    body: Option<Value>,
) -> u16 {
    let mut rb = c.request(method, url);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    if let Some(ip) = ip {
        rb = rb.header("X-Real-IP", ip);
    }
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    let r = rb.send().await.expect("请求发送失败");
    r.status().as_u16()
}

async fn count(pool: &MySqlPool, sql: &str, bind: &str) -> i64 {
    sqlx::query_scalar(sql).bind(bind).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn anonymous_logs_are_budgeted_per_ip() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let c = reqwest::Client::new();
    let pool = MySqlPool::connect(&dsn).await.unwrap();
    let token = common::login(&base, "admin", "admin123").await.expect("超管登录");

    // ── 1. 匿名写审计：同一 IP 刷 25 次，只留上限内的 20 条 ──────────────
    for _ in 0..25 {
        let st = call(
            &c,
            Method::POST,
            api("/gate-probe"),
            None,
            Some("203.0.113.20"),
            Some(json!({"probe": true})),
        )
        .await;
        assert_eq!(st, 404, "未知路径照常 404（采样只影响留档）");
    }
    let n = count(
        &pool,
        "SELECT COUNT(*) FROM audit_log WHERE path = '/api/v1/gate-probe' AND ip = ?",
        "203.0.113.20",
    )
    .await;
    assert_eq!(n, 20, "匿名写审计每窗口上限 20 条（刷 25 次不能全落库）");

    // 另一个来源有自己的额度（采样按 IP 分桶，不是全局只留一条）
    call(&c, Method::POST, api("/gate-probe"), None, Some("203.0.113.21"), None).await;
    let other = count(
        &pool,
        "SELECT COUNT(*) FROM audit_log WHERE path = '/api/v1/gate-probe' AND ip = ?",
        "203.0.113.21",
    )
    .await;
    assert_eq!(other, 1, "不同 IP 独立计数");

    // ── 2. 已鉴权写审计不采样：同样的刷法 25 条全留 ────────────────────
    for _ in 0..25 {
        let st = call(
            &c,
            Method::POST,
            api("/gate-auth-probe"),
            Some(&token),
            Some("203.0.113.22"),
            None,
        )
        .await;
        assert_eq!(st, 404);
    }
    let auth_n = count(
        &pool,
        "SELECT COUNT(*) FROM audit_log WHERE path = '/api/v1/gate-auth-probe' AND ip = ?",
        "203.0.113.22",
    )
    .await;
    assert_eq!(auth_n, 25, "已鉴权的业务写必须逐条留痕，不能被匿名采样挤掉");

    // ── 3. 登录失败：同 IP 刷 25 次只落 20 条；成功登录全量照写 ─────────
    for i in 0..25 {
        let st = call(
            &c,
            Method::POST,
            api("/auth/login"),
            None,
            Some("203.0.113.30"),
            Some(json!({"username": format!("gate_ghost_{i}"), "password": "wrong"})),
        )
        .await;
        assert!(st == 400 || st == 429, "第 {i} 次失败登录: {st}");
    }
    let fails = count(
        &pool,
        "SELECT COUNT(*) FROM login_log WHERE ip = ? AND status = 0",
        "203.0.113.30",
    )
    .await;
    assert_eq!(fails, 20, "失败登录每窗口上限 20 条");

    for _ in 0..2 {
        let st = call(
            &c,
            Method::POST,
            api("/auth/login"),
            None,
            Some("203.0.113.31"),
            Some(json!({"username": "admin", "password": "admin123"})),
        )
        .await;
        assert_eq!(st, 200, "成功登录不能被采样影响");
    }
    let ok = count(
        &pool,
        "SELECT COUNT(*) FROM login_log WHERE ip = ? AND status = 1",
        "203.0.113.31",
    )
    .await;
    assert_eq!(ok, 2, "成功登录日志逐条保留");

    // ── 4. 安全扫描命中：同 IP 刷 25 次只落 20 条，拦截判定不受影响 ─────
    for _ in 0..25 {
        let st = call(
            &c,
            Method::GET,
            format!("{base}/api/v1/health?q={SQLI_QUERY}"),
            None,
            Some("203.0.113.40"),
            None,
        )
        .await;
        assert_eq!(st, 200, "off 模式照常放行（采样只影响留档）");
    }
    let scan_n = count(
        &pool,
        "SELECT COUNT(*) FROM audit_log WHERE action = '安全扫描命中' AND ip = ?",
        "203.0.113.40",
    )
    .await;
    assert_eq!(scan_n, 20, "安全扫描命中每窗口上限 20 条");

    call(
        &c,
        Method::GET,
        format!("{base}/api/v1/health?q={SQLI_QUERY}"),
        None,
        Some("203.0.113.41"),
        None,
    )
    .await;
    let scan_other = count(
        &pool,
        "SELECT COUNT(*) FROM audit_log WHERE action = '安全扫描命中' AND ip = ?",
        "203.0.113.41",
    )
    .await;
    assert_eq!(scan_other, 1, "扫描留痕也按 IP 独立计数");
}
