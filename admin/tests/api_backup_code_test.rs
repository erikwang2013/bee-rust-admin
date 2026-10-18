// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! F4-1：TOTP 备用码（生成 / 重新生成 / 登录分流 / 一次性消费 / 错码计数）。
//! 起真实进程 + 真库（bee_admin_test），需要 `BEE_ADMIN_DB_DSN`。
//!
//! 测试侧的 TOTP 按 RFC 4226 §5.3 独立实现（只吃服务端返回的 base32 密钥），
//! 备用码只用接口返回的明文，不 import 服务端代码。
mod common;

use hmac::{Hmac, Mac};
use reqwest::Method;
use serde_json::{Value, json};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

fn client(ua: &str) -> reqwest::Client {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(reqwest::header::USER_AGENT, ua.parse().unwrap());
    reqwest::Client::builder().default_headers(h).build().unwrap()
}

fn client_with_ip(ua: &str, ip: &str) -> reqwest::Client {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(reqwest::header::USER_AGENT, ua.parse().unwrap());
    h.insert("x-real-ip", ip.parse().unwrap());
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

fn code_at(secret_b32: &str, unix: u64) -> String {
    let key = b32_decode(secret_b32);
    let mut mac = HmacSha1::new_from_slice(&key).expect("HMAC 任意长度密钥");
    mac.update(&(unix / 30).to_be_bytes());
    let d = mac.finalize().into_bytes();
    let off = (d[19] & 0x0f) as usize;
    let bin = u32::from_be_bytes([d[off] & 0x7f, d[off + 1], d[off + 2], d[off + 3]]);
    format!("{:06}", bin % 1_000_000)
}

fn code(secret: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    code_at(secret, now)
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

async fn login(
    c: &reqwest::Client,
    base: &str,
    user: &str,
    pass: &str,
    code: Option<&str>,
) -> (u16, Value) {
    let mut body = json!({ "username": user, "password": pass });
    if let Some(code) = code {
        body["code"] = json!(code);
    }
    call(c, Method::POST, format!("{base}/api/v1/auth/login"), None, Some(body)).await
}

async fn profile(c: &reqwest::Client, base: &str, token: &str) -> Value {
    let (st, v) = call(c, Method::GET, format!("{base}/api/v1/auth/profile"), Some(token), None).await;
    assert_eq!(st, 200, "profile: {v}");
    v["data"]["user"].clone()
}

/// 明文在库/审计/登录日志里出现的行数（负控统一口径）。
async fn plaintext_hits(pool: &sqlx::MySqlPool, secret_text: &str) -> i64 {
    let like = format!("%{secret_text}%");
    let cols: [(&str, &str); 3] = [
        ("admin_backup_code", "code_hash"),
        ("audit_log", "changes"),
        ("login_log", "msg"),
    ];
    let mut total = 0;
    for (table, col) in cols {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE {col} LIKE ?"))
            .bind(&like)
            .fetch_one(pool)
            .await
            .unwrap();
        total += n;
    }
    // 其余可能承载文本的列也扫一遍
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE msg LIKE ? OR path LIKE ?",
    )
    .bind(&like)
    .bind(&like)
    .fetch_one(pool)
    .await
    .unwrap();
    total + n
}

#[tokio::test]
async fn totp_backup_codes_generate_login_regenerate_and_lock() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let a = client("backup-dev-A");
    let b = client("backup-dev-B");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── 1. 未绑定 TOTP：先绑定（复用项 7 的两段式流程）──
    let (st, v) = login(&a, &base, "admin", "admin123", None).await;
    assert_eq!(st, 200, "{v}");
    let token = v["data"]["token"].as_str().unwrap().to_string();
    assert_eq!(profile(&a, &base, &token).await["backup_codes_left"], 0, "初始没有备用码");

    let (st, v) = call(&a, Method::POST, api("/auth/totp/setup"), Some(&token), None).await;
    assert_eq!(st, 200, "{v}");
    let secret = v["data"]["secret"].as_str().unwrap().to_string();
    let (st, v) = call(
        &a,
        Method::POST,
        api("/auth/totp/enable"),
        Some(&token),
        Some(json!({ "code": code(&secret) })),
    )
    .await;
    assert_eq!(st, 200, "绑定 TOTP: {v}");

    // ── 2. 端点鉴权与当前 TOTP 码 ──
    let (st, _) = call(
        &a,
        Method::POST,
        api("/auth/totp/backup-codes"),
        None,
        Some(json!({ "code": code(&secret) })),
    )
    .await;
    assert_eq!(st, 401, "必须登录");
    let (st, v) = call(
        &a,
        Method::POST,
        api("/auth/totp/backup-codes"),
        Some(&token),
        Some(json!({ "code": "000000" })),
    )
    .await;
    assert_eq!(st, 400, "当前 TOTP 码不对不能生成: {v}");
    assert_eq!(v["err"], "auth.bad_totp", "{v}");

    // ── 3. 生成 10 枚：形状正确、一次性回显、库里只有 64 位 hex 哈希 ──
    let (st, v) = call(
        &a,
        Method::POST,
        api("/auth/totp/backup-codes"),
        Some(&token),
        Some(json!({ "code": code(&secret) })),
    )
    .await;
    assert_eq!(st, 200, "生成备用码: {v}");
    let codes: Vec<String> = v["data"]["codes"]
        .as_array()
        .expect("codes 数组")
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    assert_eq!(codes.len(), 10, "{codes:?}");
    for c in &codes {
        assert_eq!(c.chars().count(), 10, "{c}");
        assert!(
            c.chars().all(|ch| ch.is_ascii_uppercase() || ('2'..='9').contains(&ch)),
            "字母表是大写字母 + 2-9: {c}"
        );
        assert!(
            !c.contains(['0', 'O', '1', 'I']),
            "必须剔除易混字符 0/O/1/I: {c}"
        );
    }
    let mut uniq = codes.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 10, "一披里不能重复: {codes:?}");

    let hashes: Vec<String> = sqlx::query_scalar(
        "SELECT code_hash FROM admin_backup_code \
         WHERE admin_id = (SELECT id FROM admin WHERE username = 'admin')",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(hashes.len(), 10, "库里 10 行");
    for h in &hashes {
        assert_eq!(h.len(), 64, "SHA-256 hex: {h}");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()), "{h}");
    }
    for c in &codes {
        assert_eq!(plaintext_hits(&pool, c).await, 0, "库里不能出现明文备用码 {c}");
    }
    assert_eq!(profile(&a, &base, &token).await["backup_codes_left"], 10, "剩余 10 枚");
    // profile / 生成响应之外的口径：登录响应里也不该带备用码
    let (_, login_v) = login(&b, &base, "admin", "admin123", Some(&codes[0])).await;
    let login_raw = login_v.to_string();
    for c in &codes {
        assert!(!login_raw.contains(c), "登录响应里不能回显备用码");
    }

    // ── 4. 用备用码登录成功；同一枚不能复用 ──
    assert_eq!(login_v["data"]["token"].is_string(), true, "备用码登录成功: {login_v}");
    assert_eq!(profile(&a, &base, &token).await["backup_codes_left"], 9, "用掉一枚");
    let (st, v) = login(&b, &base, "admin", "admin123", Some(&codes[0])).await;
    assert_eq!(st, 400, "同一枚备用码不能复用: {v}");
    assert_eq!(v["err"], "auth.bad_totp", "{v}");
    let left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_backup_code \
         WHERE admin_id = (SELECT id FROM admin WHERE username = 'admin') AND used_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 9, "失败复用不能把码再销一次");
    // 已消费的码在「未使用」集合里查不到，与未知码同一分支：都记「备用码错误」
    let replay_log: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_log WHERE msg = '备用码错误'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(replay_log, 1, "重放要留登录日志");

    // 未命中的 10 位码：报错 + 计入失败（日志留痕）
    let (st, v) = login(&b, &base, "admin", "admin123", Some("ZZZZZZZZZZ")).await;
    assert_eq!(st, 400, "错备用码: {v}");
    assert_eq!(v["err"], "auth.bad_totp", "{v}");
    let bad_log: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_log WHERE msg = '备用码错误'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(bad_log, 2, "错码要留登录日志");

    // ── 5. 重新生成：旧码全部作废，新码可用 ──
    let (st, v) = call(
        &a,
        Method::POST,
        api("/auth/totp/backup-codes"),
        Some(&token),
        Some(json!({ "code": code(&secret) })),
    )
    .await;
    assert_eq!(st, 200, "重新生成: {v}");
    let new_codes: Vec<String> = v["data"]["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_backup_code \
         WHERE admin_id = (SELECT id FROM admin WHERE username = 'admin')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(total, 10, "重新生成后库里仍是 10 行（旧码整批删除）");
    // 旧码里没用过的那枚也不能再登
    let old_unused = codes[1].clone();
    let (st, v) = login(&b, &base, "admin", "admin123", Some(&old_unused)).await;
    assert_eq!(st, 400, "旧码必须作废: {v}");
    let (st, v) = login(&b, &base, "admin", "admin123", Some(&new_codes[0])).await;
    assert_eq!(st, 200, "新码可登录: {v}");
    assert_eq!(profile(&a, &base, &token).await["backup_codes_left"], 9, "新码用掉一枚");

    // ── 5b. 并发用同一枚备用码：CAS 只有一个赢家 ──
    // Barrier 把两边对齐到同一个起跑线再发请求：不是「谁先跑完谁后跑」的接力，
    // 而是两个会话同时提交（断言不变，确定性来自起跑线而不是调度运气）。
    let ca = client_with_ip("backup-race-A", "10.9.9.1");
    let cb = client_with_ip("backup-race-B", "10.9.9.2");
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let race = |c: reqwest::Client, gate: std::sync::Arc<tokio::sync::Barrier>, base: String, code: String| async move {
        gate.wait().await;
        login(&c, &base, "admin", "admin123", Some(&code)).await
    };
    let (ra, rb) = tokio::join!(
        race(ca, barrier.clone(), base.clone(), new_codes[1].clone()),
        race(cb, barrier, base.clone(), new_codes[1].clone()),
    );
    let wins = [ra.0, rb.0].iter().filter(|s| **s == 200).count();
    assert_eq!(wins, 1, "同一枚备用码并发登录只能有一个成功: {ra:?} | {rb:?}");
    let loser = if ra.0 == 200 { &rb } else { &ra };
    assert_eq!(loser.0, 400, "输家按错码/已使用处理: {loser:?}");
    assert_eq!(loser.1["err"], "auth.bad_totp", "{loser:?}");
    assert_eq!(
        profile(&a, &base, &token).await["backup_codes_left"],
        8,
        "并发只销掉一枚"
    );

    // ── 6. 未绑定 TOTP 的账号：端点 400 ──
    let (st, v) = call(
        &a,
        Method::POST,
        api("/admins"),
        Some(&token),
        Some(json!({ "username": "nobackup", "password": "nobackup123", "nickname": "未绑定" })),
    )
    .await;
    assert_eq!(st, 200, "造第二个管理员: {v}");
    let c = client("backup-dev-C");
    let (st, v) = login(&c, &base, "nobackup", "nobackup123", None).await;
    assert_eq!(st, 200, "{v}");
    let token_c = v["data"]["token"].as_str().unwrap().to_string();
    let (st, v) = call(
        &c,
        Method::POST,
        api("/auth/totp/backup-codes"),
        Some(&token_c),
        Some(json!({ "code": "123456" })),
    )
    .await;
    assert_eq!(st, 400, "未绑定 TOTP 不能生成备用码: {v}");
    assert_eq!(v["err"], "auth.totp_not_bound", "{v}");

    // ── 7. 错备用码计数：5 次错码后正常码也被限流（429）──
    // admin 的用户桶前面已经记了 3 次失败；另起一个干净账号（nobackup）绑 TOTP、生成备用码，
    // 用独立 IP（默认 IP 桶不能被刷爆，否则后面的用例全被 429）。
    let (st, v) = call(&c, Method::POST, api("/auth/totp/setup"), Some(&token_c), None).await;
    assert_eq!(st, 200, "{v}");
    let secret_c = v["data"]["secret"].as_str().unwrap().to_string();
    let (st, v) = call(
        &c,
        Method::POST,
        api("/auth/totp/enable"),
        Some(&token_c),
        Some(json!({ "code": code(&secret_c) })),
    )
    .await;
    assert_eq!(st, 200, "nobackup 绑定 TOTP: {v}");
    let (st, v) = call(
        &c,
        Method::POST,
        api("/auth/totp/backup-codes"),
        Some(&token_c),
        Some(json!({ "code": code(&secret_c) })),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    let throttle_codes: Vec<String> = v["data"]["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();

    let attacker = client_with_ip("backup-dev-E", "10.8.8.8");
    for i in 0..5 {
        let (st, v) = login(&attacker, &base, "nobackup", "nobackup123", Some("YYYYYYYYYY")).await;
        assert_eq!(st, 400, "第 {i} 次错备用码应 400: {v}");
        assert_eq!(v["err"], "auth.bad_totp", "{v}");
    }
    let (st, v) = login(&attacker, &base, "nobackup", "nobackup123", Some(&throttle_codes[0])).await;
    assert_eq!(st, 429, "错码计数生效，正确码也先被限流: {v}");
    assert_eq!(v["err"], "auth.throttled", "{v}");

    // ── 8. 「生成备用码 / 解绑」当前码确认限流（M4）：5 次失败 / 5 分钟 → 429 ──
    let (st, v) = call(
        &a,
        Method::POST,
        api("/admins"),
        Some(&token),
        Some(json!({ "username": "confirm5", "password": "confirm12345", "nickname": "限流确认" })),
    )
    .await;
    assert_eq!(st, 200, "造限流账号: {v}");
    let c8 = client("backup-dev-F");
    let (st, v) = login(&c8, &base, "confirm5", "confirm12345", None).await;
    assert_eq!(st, 200, "{v}");
    let token8 = v["data"]["token"].as_str().unwrap().to_string();
    let (st, v) = call(&c8, Method::POST, api("/auth/totp/setup"), Some(&token8), None).await;
    assert_eq!(st, 200, "{v}");
    let secret8 = v["data"]["secret"].as_str().unwrap().to_string();
    let (st, v) = call(
        &c8,
        Method::POST,
        api("/auth/totp/enable"),
        Some(&token8),
        Some(json!({ "code": code(&secret8) })),
    )
    .await;
    assert_eq!(st, 200, "绑定 TOTP: {v}");

    let wrong = if code(&secret8) == "000000" { "000001" } else { "000000" };
    for i in 0..5 {
        let (st, v) = call(
            &c8,
            Method::POST,
            api("/auth/totp/backup-codes"),
            Some(&token8),
            Some(json!({ "code": wrong })),
        )
        .await;
        assert_eq!(st, 400, "第 {} 次错码应 400: {v}", i + 1);
        assert_eq!(v["err"], "auth.bad_totp", "{v}");
    }
    let (st, v) = call(
        &c8,
        Method::POST,
        api("/auth/totp/backup-codes"),
        Some(&token8),
        Some(json!({ "code": code(&secret8) })),
    )
    .await;
    assert_eq!(st, 429, "5 次失败后连正确码也要限流: {v}");
    assert_eq!(v["err"], "auth.throttled", "{v}");
    // 同一条确认路径的另一个端点共享同一个 per-admin 桶（解绑也必须是 429，不能放行）
    let (st, v) = call(
        &c8,
        Method::POST,
        api("/auth/totp/disable"),
        Some(&token8),
        Some(json!({ "code": code(&secret8) })),
    )
    .await;
    assert_eq!(st, 429, "解绑端点也受同一个桶限制: {v}");
    assert_eq!(v["err"], "auth.throttled", "{v}");
}
