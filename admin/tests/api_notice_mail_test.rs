// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! E2-6：公告发布的**邮件推送**（`notify.email_notice_enabled`）。
//!
//! 起真实进程 + 真库（bee_admin_test），SMTP 是本文件里现起的**本机假服务**：断言的是
//! 「服务端真的往外发了几封、发给谁」，不是自己跟自己比。假 SMTP 只走明文
//! （`smtp_encryption = none`）—— 加密传输本身是 lettre 自己的测试范围（与
//! `api_notify_test.rs` 同一取舍）。
mod common;

use reqwest::Method;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// 假 SMTP 里配的口令：故意长得不像任何别的东西，方便在日志/响应里全文搜它。
const SMTP_PW: &str = "MailPw-notice-7713-x";

// ── 假 SMTP（本机 TCP + 最小 SMTP 对话） ─────────────────────

#[derive(Clone, Default)]
struct Smtp {
    /// 服务器收到的每一行（RCPT TO 就是收件人，一封一条）
    lines: Arc<Mutex<Vec<String>>>,
}

impl Smtp {
    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
}

/// 收件人列表（按投递顺序，一封一条 `RCPT TO:<addr>`）。
fn rcpts(st: &Smtp) -> Vec<String> {
    st.lines()
        .into_iter()
        .filter(|l| l.to_uppercase().starts_with("RCPT TO:"))
        .map(|l| l.split('<').nth(1).unwrap_or("").trim_end_matches('>').to_string())
        .collect()
}

async fn fake_smtp() -> (Smtp, u16) {
    let st = Smtp::default();
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let srv = st.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = l.accept().await {
            let st = srv.clone();
            tokio::spawn(async move { serve_smtp(sock, st).await });
        }
    });
    (st, port)
}

/// 只说 lettre 需要的那几句：EHLO →（有凭据时）AUTH PLAIN → MAIL/RCPT/DATA/QUIT。
async fn serve_smtp(sock: tokio::net::TcpStream, st: Smtp) {
    let (r, mut w) = sock.into_split();
    let mut lines = BufReader::new(r).lines();
    let _ = w.write_all(b"220 fake ESMTP\r\n").await;
    let _ = w.flush().await;
    while let Ok(Some(line)) = lines.next_line().await {
        st.lines.lock().unwrap().push(line.clone());
        let up = line.to_uppercase();
        let reply: &[u8] = if up.starts_with("EHLO") || up.starts_with("HELO") {
            b"250-fake\r\n250-AUTH PLAIN\r\n250 OK\r\n"
        } else if up.starts_with("DATA") {
            b"354 End data with <CR><LF>.<CR><LF>\r\n"
        } else if up.starts_with("QUIT") {
            b"221 Bye\r\n"
        } else if up.starts_with("AUTH") {
            b"235 2.7.0 Authentication successful\r\n"
        } else {
            b"250 OK\r\n"
        };
        let _ = w.write_all(reply).await;
        let _ = w.flush().await;
        if up.starts_with("DATA") {
            let mut body = String::new();
            while let Ok(Some(l)) = lines.next_line().await {
                if l == "." {
                    break;
                }
                body.push_str(&l);
                body.push('\n');
            }
            st.lines.lock().unwrap().push(format!("[BODY] {body}"));
            let _ = w.write_all(b"250 OK\r\n").await;
            let _ = w.flush().await;
        }
        if up.starts_with("QUIT") {
            return;
        }
    }
}

// ── HTTP 辅助 ───────────────────────────────────────────────

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
    (status, r.json().await.unwrap_or(Value::Null))
}

async fn publish(c: &reqwest::Client, base: &str, token: &str, title: &str) -> (u16, Value) {
    call(
        c,
        Method::POST,
        format!("{base}/api/v1/notices"),
        Some(token),
        Some(json!({ "title": title, "content": "正文", "status": 1 })),
    )
    .await
}

/// 事件与邮件都是**异步**的：响应返回不代表已经发出去，等条件成立再断言。
async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ok() {
        assert!(Instant::now() < deadline, "等 {what} 超时");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 给「不该发」的断言留出足够时间：真发了的话，本机 SMTP 在毫秒级就会到。
async fn settle() {
    tokio::time::sleep(Duration::from_millis(1200)).await;
}

async fn get_config(c: &reqwest::Client, url: &str, token: &str) -> Value {
    let (st, v) = call(c, Method::GET, url.to_string(), Some(token), None).await;
    assert_eq!(st, 200, "读通知配置: {v}");
    v["data"].clone()
}

#[tokio::test]
async fn notice_published_mail_fanout_excludes_publisher_and_respects_switch() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let c = reqwest::Client::new();
    let token = common::login(&base, "admin", "admin123").await.expect("登录失败");
    let (smtp, smtp_port) = fake_smtp().await;

    // ── 1. 造收件人：发布者本人也填邮箱（排除逻辑必须靠身份，不是靠「发布者没邮箱」）
    let (st, v) = call(
        &c,
        Method::PUT,
        api("/auth/profile"),
        Some(&token),
        Some(json!({ "nickname": "超管", "email": "publisher@test.local", "phone": "" })),
    )
    .await;
    assert_eq!(st, 200, "给发布者填邮箱: {v}");
    for (u, e, status) in [
        ("mailok1", "ok1@test.local", 1),  // 收
        ("mailok2", "ok2@test.local", 1),  // 收
        ("mailoff", "off@test.local", 0),  // 禁用 → 不收
        ("mailnone", "", 1),               // 没填邮箱 → 不收
    ] {
        let (st, v) = call(
            &c,
            Method::POST,
            api("/admins"),
            Some(&token),
            Some(json!({
                "username": u, "password": "Passw0rd!23", "nickname": u,
                "email": e, "status": status,
            })),
        )
        .await;
        assert_eq!(st, 200, "建账号 {u}: {v}");
    }

    // ── 2. 默认关：GET 回包必须带上这个字段（少一个 = 保存一次就静默重置）
    let cfg = get_config(&c, &api("/notify/config"), &token).await;
    assert_eq!(cfg["email_notice_enabled"], false, "默认关（sys_config 默认 0）: {cfg}");

    // ── 3. 配好 SMTP + 打开公告邮件
    let mut cfg = json!({
        "webhook_enabled": false,
        "webhook_url": "",
        "smtp_enabled": true,
        "smtp_host": "127.0.0.1",
        "smtp_port": smtp_port,
        "smtp_user": "notice-mailer@test.local",
        "smtp_password": SMTP_PW,
        "smtp_from": "bee-admin@test.local",
        "smtp_encryption": "none",
        "email_notice_enabled": true,
    });
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(cfg.clone())).await;
    assert_eq!(st, 200, "存通知配置: {v}");
    let got = get_config(&c, &api("/notify/config"), &token).await;
    assert_eq!(got["email_notice_enabled"], true, "GET 要回开关: {got}");

    // ── 4. 发布公告 → 恰好两封：两个启用且有邮箱的账号，**不含发布者**
    let (st, v) = publish(&c, &base, &token, "邮件推送验收公告").await;
    assert_eq!(st, 200, "发公告: {v}");
    wait_until("两封公告邮件", || rcpts(&smtp).len() >= 2).await;
    settle().await;
    let mut got = rcpts(&smtp);
    got.sort();
    assert_eq!(
        got,
        vec!["ok1@test.local".to_string(), "ok2@test.local".to_string()],
        "收件人 = 全部启用且填了邮箱的账号，排除发布者；禁用与没填邮箱的都跳过: {got:?}"
    );

    // ── 5. 关掉开关 → 不发（负控：不是「有公告就发」）
    cfg["email_notice_enabled"] = json!(false);
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(cfg.clone())).await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = publish(&c, &base, &token, "关掉后不发").await;
    assert_eq!(st, 200, "{v}");
    settle().await;
    assert_eq!(rcpts(&smtp).len(), 2, "开关关着就不发: {:?}", rcpts(&smtp));

    // ── 6. SMTP 总开关也关着 → 不发（公告邮件寄生在 SMTP 上）
    cfg["email_notice_enabled"] = json!(true);
    cfg["smtp_enabled"] = json!(false);
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(cfg.clone())).await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = publish(&c, &base, &token, "SMTP 关着不发").await;
    assert_eq!(st, 200, "{v}");
    settle().await;
    assert_eq!(rcpts(&smtp).len(), 2, "SMTP 没启用时不发: {:?}", rcpts(&smtp));

    // ── 7. 清空口令（前端「清空口令」= 拿 GET 回包整体回填、只把口令置空）→ 开关保持
    cfg["smtp_enabled"] = json!(true);
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(cfg.clone())).await;
    assert_eq!(st, 200, "{v}");
    let mut body = get_config(&c, &api("/notify/config"), &token).await;
    body.as_object_mut().unwrap().remove("smtp_password_set");
    body["smtp_password"] = json!(""); // 空串 = 清空
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(body)).await;
    assert_eq!(st, 200, "清空口令: {v}");
    let got = get_config(&c, &api("/notify/config"), &token).await;
    assert_eq!(got["smtp_password_set"], false, "口令已清空: {got}");
    assert_eq!(got["email_notice_enabled"], true, "清空口令不能把公告邮件开关一起重置: {got}");
}
