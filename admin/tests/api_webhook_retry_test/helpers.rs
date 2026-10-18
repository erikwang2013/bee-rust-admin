// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! F4 Webhook 用例的本机假服务、独立 HMAC 与 HTTP 辅助（F3 拆行数：断言零改动）。
use super::common;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use hmac::{Hmac, Mac};
use reqwest::Method;
use serde_json::{Value, json};
use sha2::Sha256;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;

/// 签名密钥：故意长得不像任何别的东西，方便全文搜它。
pub const SECRET: &str = "whsec-f4-retry-9137-x";

// ── 假 webhook：记录原始体与两个签名头 ──────────────────────

#[derive(Clone, Default)]
pub struct Hit {
    pub body: Vec<u8>,
    pub ts: Option<String>,
    pub sig: Option<String>,
}

#[derive(Clone, Default)]
pub struct Hook {
    pub hits: Arc<Mutex<Vec<Hit>>>,
    fail_first: Arc<AtomicUsize>,
    /// 设置后所有请求都回这个重定向（测「不跟随」用）。
    redirect: Arc<Mutex<Option<(StatusCode, String)>>>,
    /// 响应前先睡一会儿（测领取竞争时把窗口拉开）。
    delay_ms: Arc<AtomicU64>,
}

impl Hook {
    pub fn count(&self) -> usize {
        self.hits.lock().unwrap().len()
    }
    pub fn hit(&self, i: usize) -> Hit {
        self.hits.lock().unwrap()[i].clone()
    }
    pub fn fail_first(&self, n: usize) {
        self.fail_first.store(n, Ordering::SeqCst);
    }
    pub fn redirect(&self, status: StatusCode, to: &str) {
        *self.redirect.lock().unwrap() = Some((status, to.to_string()));
    }
    pub fn clear_redirect(&self) {
        *self.redirect.lock().unwrap() = None;
    }
    pub fn delay_ms(&self, ms: u64) {
        self.delay_ms.store(ms, Ordering::SeqCst);
    }
}

async fn hook_handler(State(h): State<Hook>, headers: HeaderMap, body: Bytes) -> Response {
    h.hits.lock().unwrap().push(Hit {
        body: body.to_vec(),
        ts: headers.get("x-bra-timestamp").and_then(|v| v.to_str().ok()).map(str::to_string),
        sig: headers.get("x-bra-signature").and_then(|v| v.to_str().ok()).map(str::to_string),
    });
    let delay = h.delay_ms.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    if let Some((status, to)) = h.redirect.lock().unwrap().clone() {
        return (status, [(axum::http::header::LOCATION, to)]).into_response();
    }
    let failed = h
        .fail_first
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok();
    if failed {
        return (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response();
    }
    (StatusCode::OK, "ok").into_response()
}

pub async fn fake_webhook() -> (Hook, u16) {
    let h = Hook::default();
    let app = Router::new().route("/hook", post(hook_handler)).with_state(h.clone());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(l, app).await;
    });
    (h, port)
}

// ── 独立 HMAC（不复用服务端代码） ────────────────────────────

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub fn sign(secret: &str, ts: u64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(ts.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!("sha256={}", hex_lower(&mac.finalize().into_bytes()))
}

/// 收到的那一条必须带合法的 time/签名（签发时间 ±5 分钟内）。
pub fn verify_signed(hit: &Hit, secret: &str) {
    let ts: u64 = hit
        .ts
        .as_deref()
        .expect("配置了密钥就必须有 X-BRA-Timestamp")
        .parse()
        .expect("时间戳是 unix 秒");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(now.abs_diff(ts) < 300, "时间戳偏得太远: {ts} vs {now}");
    let got = hit.sig.as_deref().expect("配置了密钥就必须有 X-BRA-Signature");
    assert_eq!(got, sign(secret, ts, &hit.body), "签名必须是 HMAC-SHA256(ts.body)");
}

// ── HTTP 辅助 ───────────────────────────────────────────────

pub async fn call(
    c: &reqwest::Client,
    method: Method,
    url: String,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, String) {
    let mut rb = c.request(method, url);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    let r = rb.send().await.expect("请求发送失败");
    let status = r.status().as_u16();
    (status, r.text().await.unwrap_or_default())
}

pub fn cfg_json(port: u16, secret: Option<&str>) -> Value {
    let mut v = json!({
        "webhook_enabled": true,
        "webhook_url": format!("http://127.0.0.1:{port}/hook"),
        "smtp_enabled": false,
        "smtp_host": "",
        "smtp_port": 587,
        "smtp_user": "",
        "smtp_from": "",
        "smtp_encryption": "none",
        "email_notice_enabled": false,
    });
    if let Some(s) = secret {
        v["webhook_secret"] = json!(s);
    }
    v
}

pub async fn save_cfg(c: &reqwest::Client, base: &str, token: &str, body: Value) -> (u16, String) {
    call(c, Method::PUT, format!("{base}/api/v1/notify/config"), Some(token), Some(body)).await
}

pub async fn publish(c: &reqwest::Client, base: &str, token: &str, title: &str) {
    let (st, raw) = call(
        c,
        Method::POST,
        format!("{base}/api/v1/notices"),
        Some(token),
        Some(json!({ "title": title, "content": "正文", "status": 1 })),
    )
    .await;
    assert_eq!(st, 200, "发公告失败: {raw}");
}

/// 事件是异步发的：等条件成立。
pub async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ok() {
        assert!(Instant::now() < deadline, "等 {what} 超时");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn wait_row(pool: &sqlx::MySqlPool, title: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notify_delivery WHERE payload LIKE ?")
            .bind(format!("%{title}%"))
            .fetch_one(pool)
            .await
            .unwrap();
        if n > 0 {
            return;
        }
        assert!(Instant::now() < deadline, "等 {title} 的失败记录超时");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub type Row = (i64, String, i32, Option<chrono::NaiveDateTime>, String);

pub async fn delivery(pool: &sqlx::MySqlPool, title: &str) -> Row {
    sqlx::query_as(
        "SELECT id, status, attempts, next_at, msg FROM notify_delivery \
         WHERE payload LIKE ? ORDER BY id DESC LIMIT 1",
    )
    .bind(format!("%{title}%"))
    .fetch_one(pool)
    .await
    .unwrap()
}

pub async fn make_due(pool: &sqlx::MySqlPool, id: i64) {
    // 测试侧算时间：sqlx 会话时区是 UTC（MySQL NOW() 会差 8 小时）
    let past = chrono::Utc::now().naive_utc() - chrono::Duration::seconds(5);
    sqlx::query("UPDATE notify_delivery SET next_at = ? WHERE id = ?")
        .bind(past)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

/// 手动跑一趟 `notify_retry`（接口异步），返回这一趟的 job_log 摘要。
pub async fn run_retry(
    c: &reqwest::Client,
    base: &str,
    pool: &sqlx::MySqlPool,
    token: &str,
) -> String {
    let id: i64 = sqlx::query_scalar("SELECT id FROM job WHERE code = 'notify_retry'")
        .fetch_one(pool)
        .await
        .expect("notify_retry 必须被种子注册");
    let last = common::last_log_id(pool, "notify_retry").await;
    common::trigger(c, base, &common::enc_id(id as u64), token).await;
    let (status, msg, _) = common::wait_job_log(pool, "notify_retry", last).await;
    assert_eq!(status, 1, "notify_retry 应成功: {msg}");
    msg
}
