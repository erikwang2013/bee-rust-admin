// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! F4-2 / F4-3 / F4-4：Webhook HMAC 签名、失败落库重投、发送时 DNS 封禁。
//! 起真实进程 + 真库（bee_admin_test），需要 `BEE_ADMIN_DB_DSN`。
//!
//! 假 webhook 收**原始字节**与头：签名必须由测试侧独立 HMAC 复算，不能只看
//! 「头存在」。重投通过 `POST /api/v1/jobs/{id}/run` 手动触发（接口是异步的，
//! 用 `job_log` 判定这一趟跑完）。
mod common;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use hmac::{Hmac, Mac};
use reqwest::Method;
use serde_json::{Value, json};
use sha2::Sha256;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;

/// 签名密钥：故意长得不像任何别的东西，方便全文搜它。
const SECRET: &str = "whsec-f4-retry-9137-x";

// ── 假 webhook：记录原始体与两个签名头 ──────────────────────

#[derive(Clone, Default)]
struct Hit {
    body: Vec<u8>,
    ts: Option<String>,
    sig: Option<String>,
}

#[derive(Clone, Default)]
struct Hook {
    hits: Arc<Mutex<Vec<Hit>>>,
    fail_first: Arc<AtomicUsize>,
}

impl Hook {
    fn count(&self) -> usize {
        self.hits.lock().unwrap().len()
    }
    fn hit(&self, i: usize) -> Hit {
        self.hits.lock().unwrap()[i].clone()
    }
    fn fail_first(&self, n: usize) {
        self.fail_first.store(n, Ordering::SeqCst);
    }
}

async fn hook_handler(State(h): State<Hook>, headers: HeaderMap, body: Bytes) -> (StatusCode, &'static str) {
    h.hits.lock().unwrap().push(Hit {
        body: body.to_vec(),
        ts: headers.get("x-bra-timestamp").and_then(|v| v.to_str().ok()).map(str::to_string),
        sig: headers.get("x-bra-signature").and_then(|v| v.to_str().ok()).map(str::to_string),
    });
    let failed = h
        .fail_first
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok();
    if failed {
        return (StatusCode::INTERNAL_SERVER_ERROR, "boom");
    }
    (StatusCode::OK, "ok")
}

async fn fake_webhook() -> (Hook, u16) {
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

fn sign(secret: &str, ts: u64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(ts.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!("sha256={}", hex_lower(&mac.finalize().into_bytes()))
}

/// 收到的那一条必须带合法的 time/签名（签发时间 ±5 分钟内）。
fn verify_signed(hit: &Hit, secret: &str) {
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

async fn call(
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

fn cfg_json(port: u16, secret: Option<&str>) -> Value {
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

async fn save_cfg(c: &reqwest::Client, base: &str, token: &str, body: Value) -> (u16, String) {
    call(c, Method::PUT, format!("{base}/api/v1/notify/config"), Some(token), Some(body)).await
}

async fn publish(c: &reqwest::Client, base: &str, token: &str, title: &str) {
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
async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ok() {
        assert!(Instant::now() < deadline, "等 {what} 超时");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_row(pool: &sqlx::MySqlPool, title: &str) {
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

type Row = (i64, String, i32, Option<chrono::NaiveDateTime>, String);

async fn delivery(pool: &sqlx::MySqlPool, title: &str) -> Row {
    sqlx::query_as(
        "SELECT id, status, attempts, next_at, msg FROM notify_delivery \
         WHERE payload LIKE ? ORDER BY id DESC LIMIT 1",
    )
    .bind(format!("%{title}%"))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn make_due(pool: &sqlx::MySqlPool, id: i64) {
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
async fn run_retry(c: &reqwest::Client, base: &str, pool: &sqlx::MySqlPool, token: &str) -> String {
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

#[tokio::test]
async fn webhook_signs_failures_persist_and_retry() {
    // 派单要求「测试侧独立 HMAC 验签（含已知向量）」。这条向量与 python hmac 算的一致。
    assert_eq!(
        sign("secret-key", 1_700_000_000, br#"{"event":"notice.published"}"#),
        "sha256=1e5c6eb66be401446bb836769daae2d22a1d2e73cfbfdff272fc7b7e2a159082"
    );

    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let c = reqwest::Client::new();
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let token = common::login(&base, "admin", "admin123").await.expect("登录失败");
    let (hook, hook_port) = fake_webhook().await;

    // 先把调度器启动后的首趟 notify_retry 让过去（否则它可能在用例中途抢跑/占闸）
    let (st, msg, _) = common::wait_job_log(&pool, "notify_retry", 0).await;
    assert_eq!(st, 1, "启动后的首趟 notify_retry: {msg}");

    // ── 1. 密钥三态与落库加密 ────────────────────────────────
    let (st, raw) = call(&c, Method::GET, api("/notify/config"), Some(&token), None).await;
    assert_eq!(st, 200);
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["data"]["webhook_secret_set"], false, "默认未设置: {v}");

    let (st, raw) = save_cfg(&c, &base, &token, cfg_json(hook_port, Some(SECRET))).await;
    assert_eq!(st, 200, "存密钥: {raw}");
    assert!(!raw.contains(SECRET), "保存响应不能回显密钥: {raw}");

    let (st, raw) = call(&c, Method::GET, api("/notify/config"), Some(&token), None).await;
    assert_eq!(st, 200);
    assert!(!raw.contains(SECRET), "GET 不能回显密钥: {raw}");
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["data"]["webhook_secret_set"], true, "只回已设置标记: {v}");

    let stored: String = sqlx::query_scalar("SELECT value FROM sys_config WHERE config_key = 'notify.webhook_secret'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(stored, SECRET, "落库不能是明文");
    assert!(!stored.contains(SECRET), "密文不能含明文: {stored}");
    assert!(stored.len() > SECRET.len(), "密文应长于明文");
    let public: i8 = sqlx::query_scalar("SELECT is_public FROM sys_config WHERE config_key = 'notify.webhook_secret'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(public, 0, "密钥键必须私有");

    // 审计只记「改过」，且全文无密钥（看的是上面那次带密钥的保存）
    let changes: String = sqlx::query_scalar(
        "SELECT changes FROM audit_log WHERE path LIKE '%/notify/config' AND method = 'PUT' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let changes: Value = serde_json::from_str(&changes).unwrap();
    assert_eq!(changes["webhook_secret"]["changed"], true, "{changes}");
    assert_eq!(changes["webhook_secret"]["old"], Value::Null, "不能记旧值: {changes}");
    let leaked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE changes LIKE ? OR msg LIKE ? OR path LIKE ?",
    )
    .bind(format!("%{SECRET}%"))
    .bind(format!("%{SECRET}%"))
    .bind(format!("%{SECRET}%"))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0, "审计里不能出现密钥");

    // 缺省 = 不改
    let (st, _) = save_cfg(&c, &base, &token, cfg_json(hook_port, None)).await;
    assert_eq!(st, 200);
    let (_, raw) = call(&c, Method::GET, api("/notify/config"), Some(&token), None).await;
    assert!(raw.contains("\"webhook_secret_set\":true"), "缺省不能清掉密钥: {raw}");

    // ── 2. 正常投递：测试侧独立验签 ──────────────────────────
    let before = hook.count();
    publish(&c, &base, &token, "签名验收公告").await;
    wait_until("签名投递", || hook.count() >= before + 1).await;
    let hit = hook.hit(before);
    verify_signed(&hit, SECRET);
    let body: Value = serde_json::from_slice(&hit.body).unwrap();
    assert_eq!(body["event"], "notice.published", "{body}");
    assert_eq!(body["data"]["title"], "签名验收公告", "{body}");

    // ── 3. 空串 = 清空：之后不签名 ───────────────────────────
    let (st, _) = save_cfg(&c, &base, &token, cfg_json(hook_port, Some(""))).await;
    assert_eq!(st, 200);
    let (_, raw) = call(&c, Method::GET, api("/notify/config"), Some(&token), None).await;
    assert!(raw.contains("\"webhook_secret_set\":false"), "空串要清空: {raw}");
    let empty: String = sqlx::query_scalar("SELECT value FROM sys_config WHERE config_key = 'notify.webhook_secret'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(empty.is_empty(), "清空后库里是空串: {empty:?}");

    let before = hook.count();
    publish(&c, &base, &token, "未签名验收公告").await;
    wait_until("未签名投递", || hook.count() >= before + 1).await;
    let hit = hook.hit(before);
    assert!(hit.ts.is_none(), "未配置密钥就不该有 X-BRA-Timestamp");
    assert!(hit.sig.is_none(), "未配置密钥就不该有 X-BRA-Signature");

    // 恢复密钥，供后面的重投用例用（重投要「复用签名」）
    let (st, _) = save_cfg(&c, &base, &token, cfg_json(hook_port, Some(SECRET))).await;
    assert_eq!(st, 200);

    // ── 4. 首发失败落 pending；手动触发重投成功（2 次尝试，重投也签名）──
    hook.fail_first(1);
    let before = hook.count();
    publish(&c, &base, &token, "重投验收-失败首发").await;
    wait_until("失败首发", || hook.count() >= before + 1).await;
    wait_row(&pool, "重投验收-失败首发").await; // 落库发生在请求返回之后
    let msg = run_retry(&c, &base, &pool, &token).await;
    assert!(msg.contains("重投 0 条"), "首发刚失败（+1 分钟后才到期），这一趟不该重投: {msg}");

    let (id, status, attempts, next_at, err) = delivery(&pool, "重投验收-失败首发").await;
    assert_eq!(status, "pending", "普通失败进 pending: {err}");
    assert_eq!(attempts, 1, "首发算一次尝试");
    assert!(next_at.is_some(), "pending 必须有 next_at");
    assert!(err.contains("HTTP 500"), "原因要留 500: {err}");

    make_due(&pool, id).await;
    let before = hook.count();
    let msg = run_retry(&c, &base, &pool, &token).await;
    assert!(msg.contains("重投 1 条：成功 1"), "这一趟应成功重投 1 条: {msg}");
    wait_until("重投到达", || hook.count() >= before + 1).await;
    verify_signed(&hook.hit(before), SECRET);
    let (_, status, attempts, next_at, err) = delivery(&pool, "重投验收-失败首发").await;
    assert_eq!(status, "delivered", "{err}");
    assert_eq!(attempts, 2, "首发 + 一次重投 = 2");
    assert!(next_at.is_none(), "投递成功不再排期");

    // ── 5. 持续失败：重投 3 次后终态 failed ──────────────────
    hook.fail_first(1_000_000);
    let before = hook.count();
    publish(&c, &base, &token, "重投验收-耗尽").await;
    wait_until("耗尽首发", || hook.count() >= before + 1).await;
    wait_row(&pool, "重投验收-耗尽").await;
    let (id, _, _, _, _) = delivery(&pool, "重投验收-耗尽").await;
    for round in 0..3 {
        make_due(&pool, id).await;
        let msg = run_retry(&c, &base, &pool, &token).await;
        let (_, status, attempts, _, _) = delivery(&pool, "重投验收-耗尽").await;
        assert_eq!(attempts, round + 2, "第 {} 次重投后的 attempts: {msg}", round + 1);
        if round < 2 {
            assert_eq!(status, "pending", "还没到上限应继续排期: {msg}");
        }
    }
    let (_, status, attempts, next_at, err) = delivery(&pool, "重投验收-耗尽").await;
    assert_eq!(status, "failed", "首次 + 3 次重投后终态: {err}");
    assert_eq!(attempts, 4, "最多 4 次尝试");
    assert!(next_at.is_none(), "终态不再重投");
    assert!(err.contains("HTTP 500"), "{err}");
    assert_eq!(hook.count(), before + 4, "首发 + 3 次重投 = 4 个请求: {}", hook.count());

    // 任务注册：种子里的间隔是 300 秒（每 5 分钟）
    let cron: String = sqlx::query_scalar("SELECT cron FROM job WHERE code = 'notify_retry'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(cron, "300", "notify_retry 每 5 分钟跑一次");

    // ── 6. 30 天清理 ─────────────────────────────────────────
    let old_id = common::new_row_id();
    let old_time = chrono::Utc::now().naive_utc() - chrono::Duration::days(31);
    sqlx::query(
        "INSERT INTO notify_delivery (id, event, payload, status, attempts, next_at, msg, created_at) \
         VALUES (?, 'notice.published', '{\"event\":\"notice.published\"}', 'failed', 4, NULL, '旧账本', ?)",
    )
    .bind(old_id)
    .bind(old_time)
    .execute(&pool)
    .await
    .unwrap();
    let msg = run_retry(&c, &base, &pool, &token).await;
    assert!(msg.contains("清理过期记录 1 条"), "30 天前的账本要清掉: {msg}");
    let old_left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notify_delivery WHERE id = ?")
        .bind(old_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(old_left, 0);

    // ── 7. 发送时 DNS 封禁：链路本地 = 终态失败，不投递、不重投 ──
    // 保存时校验只看字面量；这里绕过接口直接改库，模拟「域名或字面量被改成内网」。
    sqlx::query("UPDATE sys_config SET value = 'http://169.254.169.254/latest/meta-data' WHERE config_key = 'notify.webhook_url'")
        .execute(&pool)
        .await
        .unwrap();
    hook.fail_first(0);
    let before = hook.count();
    publish(&c, &base, &token, "DNS 终态验收").await;
    wait_row(&pool, "DNS 终态验收").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (_, status, attempts, next_at, err) = delivery(&pool, "DNS 终态验收").await;
    assert_eq!(status, "failed", "链路本地是终态: {err}");
    assert_eq!(attempts, 1, "终态不重投");
    assert!(next_at.is_none(), "终态不排期");
    assert!(err.contains("链路本地"), "原因要说清拦的是什么: {err}");
    assert_eq!(hook.count(), before, "链路本地地址绝不能收到请求");
    let msg = run_retry(&c, &base, &pool, &token).await;
    assert!(msg.contains("重投 0 条"), "终态行不该被重投: {msg}");
    assert_eq!(hook.count(), before, "重投也不许打内网");
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notify_delivery WHERE payload LIKE '%DNS 终态验收%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 1, "只落一条终态记录");
    let _ = id;

    // 密钥不出现在任何落库文本里（审计 / 登录日志 / 任务日志 / 投递账本）
    let leaked: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM audit_log WHERE changes LIKE ? OR msg LIKE ? OR path LIKE ?) \
              + (SELECT COUNT(*) FROM login_log WHERE msg LIKE ?) \
              + (SELECT COUNT(*) FROM job_log WHERE msg LIKE ?) \
              + (SELECT COUNT(*) FROM notify_delivery WHERE payload LIKE ? OR msg LIKE ?)",
    )
    .bind(format!("%{SECRET}%"))
    .bind(format!("%{SECRET}%"))
    .bind(format!("%{SECRET}%"))
    .bind(format!("%{SECRET}%"))
    .bind(format!("%{SECRET}%"))
    .bind(format!("%{SECRET}%"))
    .bind(format!("%{SECRET}%"))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0, "密钥不能出现在任何落库文本里");
}
