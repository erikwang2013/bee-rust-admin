// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 通知用例的本机假服务与 HTTP 辅助（F3 拆行数：断言零改动）。
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use reqwest::Method;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// 假 SMTP 里配的口令：故意长得不像任何别的东西，方便在库/日志/响应里全文搜它。
pub const SMTP_PW: &str = "Pw0rd-notify-9137-x";

// ── 假 webhook（本机 HTTP） ──────────────────────────────────

#[derive(Clone, Default)]
pub struct Hook {
    /// 收到的请求体。**失败的也记**：重试次数就是靠它数出来的
    hits: Arc<Mutex<Vec<Value>>>,
    /// 前 N 次回 500（用来钉「失败要重试」与重试次数上限）
    fail_first: Arc<AtomicUsize>,
}

impl Hook {
    pub fn count(&self) -> usize {
        self.hits.lock().unwrap().len()
    }
    pub fn bodies(&self) -> Vec<Value> {
        self.hits.lock().unwrap().clone()
    }
    pub fn fail_first(&self, n: usize) {
        self.fail_first.store(n, Ordering::SeqCst);
    }
}

async fn hook_handler(State(h): State<Hook>, body: Option<axum::Json<Value>>) -> (StatusCode, &'static str) {
    h.hits.lock().unwrap().push(body.map(|axum::Json(v)| v).unwrap_or(Value::Null));
    let failed = h
        .fail_first
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok();
    if failed {
        return (StatusCode::INTERNAL_SERVER_ERROR, "boom");
    }
    (StatusCode::OK, "ok")
}

/// 起一个只认 `POST /hook` 的假 webhook，返回 (状态, 端口)。
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

// ── 假 SMTP（本机 TCP + 最小 SMTP 对话） ─────────────────────

#[derive(Clone, Default)]
pub struct Smtp {
    /// 服务器收到的每一行（含 RCPT TO 与 AUTH PLAIN 的凭据）
    lines: Arc<Mutex<Vec<String>>>,
}

impl Smtp {
    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
}

pub async fn fake_smtp() -> (Smtp, u16) {
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
/// 只广告 PLAIN 一种认证机制，免得 lettre 走 LOGIN 那套多轮 base64 对话。
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
            // MAIL FROM / RCPT TO / RSET / NOOP 一律 250
            b"250 OK\r\n"
        };
        let _ = w.write_all(reply).await;
        let _ = w.flush().await;
        if up.starts_with("DATA") {
            // 收正文直到单独一行的点号（SMTP 的结束标记）
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

pub async fn call_raw(
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
    let text = r.text().await.unwrap_or_default();
    (status, text)
}

pub async fn call(
    c: &reqwest::Client,
    method: Method,
    url: String,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let (st, text) = call_raw(c, method, url, token, body).await;
    (st, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// 等条件成立：事件是**异步**发的（响应返回不代表请求已经出去）。
pub async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ok() {
        assert!(Instant::now() < deadline, "等 {what} 超时");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 让还在飞的重试都跑完再断言「恰好 N 次」——重试间隔 500ms，给足一倍余量。
pub async fn settle() {
    tokio::time::sleep(Duration::from_millis(1200)).await;
}

/// 发一条公告（`status = 1` 才是发布）。
pub async fn publish(c: &reqwest::Client, base: &str, token: &str, title: &str, status: i8) -> (u16, Value) {
    call(
        c,
        Method::POST,
        format!("{base}/api/v1/notices"),
        Some(token),
        Some(json!({ "title": title, "content": "正文", "status": status })),
    )
    .await
}

/// 明文口令在假 SMTP 的哪一行里出现过（AUTH PLAIN 的 base64 载荷）。
pub fn auth_has_password(lines: &[String]) -> bool {
    lines.iter().filter(|l| l.to_uppercase().starts_with("AUTH")).any(|l| {
        l.split_whitespace()
            .skip(2)
            .any(|tok| String::from_utf8_lossy(&B64.decode(tok).unwrap_or_default()).contains(SMTP_PW))
    })
}

pub fn collect_perms(nodes: &Value, out: &mut Vec<String>) {
    for n in nodes.as_array().into_iter().flatten() {
        if let Some(p) = n["perm"].as_str().filter(|p| !p.is_empty()) {
            out.push(p.to_string());
        }
        collect_perms(&n["children"], out);
    }
}
