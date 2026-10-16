// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 扩展工程 C2（项 7）：TOTP 动态验证码（绑定 / 解绑 / 登录校验）+ 登录设备管理
//! （列表 / 单设备踢出）。起真实进程 + 真库（bee_admin_test），需要 `BEE_ADMIN_DB_DSN`。
//!
//! 测试侧的验证码按 RFC 4226 §5.3 的算法**独立实现**（只喂服务端返回的 base32 密钥，
//! 不 import src 的代码）——验的是服务端真的认 RFC 算出来的码，不是自己跟自己比。
mod common;

use hmac::{Hmac, Mac};
use reqwest::Method;
use serde_json::{Value, json};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// 设备指纹是 UA（本项目口径）：不同 UA = 不同设备行，同一 UA 复用同一行。
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

/// 每次现算：跨过 30 秒边界也不会用到过期码（服务端还有 ±1 步窗口）。
fn code(secret: &str) -> String {
    code_at(secret, now_unix())
}

/// S1（一枚码只能换一次登录）之后：本测试要拿同一个 secret 的码连登好几次，图的是
/// 绑定/设备/踢人这些行为，不是重放（重放有 api_totp_replay_test 专门钉住），所以每回
/// 要再登一次之前把登录账本清掉，免得被当重放拒了。
async fn forget_step(pool: &sqlx::MySqlPool) {
    sqlx::query("UPDATE admin SET totp_last_step = NULL WHERE username = 'admin'")
        .execute(pool)
        .await
        .unwrap();
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

async fn profile(c: &reqwest::Client, base: &str, token: &str) -> (u16, Value) {
    call(c, Method::GET, format!("{base}/api/v1/auth/profile"), Some(token), None).await
}

/// 手签一个「升级前」的 token：payload 里**没有** `dv`（服务端解出来是 0）。
/// 密钥 / issuer / audience 与后端同源，直接读 conf/app.conf.test（同在 [jwt] 节，
/// 键写在哪个节都认），不硬编码猜 —— 猜错的症状是「一律 401」，比报缺键难查。
fn legacy_token(admin_id: i64, ver: i32) -> String {
    #[derive(serde::Serialize)]
    struct LegacyClaims {
        sub: i64,
        ver: i32,
    }
    struct NoBlacklist;
    impl jwt_rust::storage::TokenStorage for NoBlacklist {
        fn blacklist(&self, _jti: &str, _expire_time: i64) -> Result<bool, jwt_rust::JwtError> {
            Ok(true)
        }
        fn is_blacklisted(&self, _jti: &str) -> Result<bool, jwt_rust::JwtError> {
            Ok(false)
        }
        fn cleanup(&self) -> Result<bool, jwt_rust::JwtError> {
            Ok(true)
        }
    }
    let conf = std::fs::read_to_string("conf/app.conf.test").unwrap();
    let val = |k: &str| {
        conf.lines()
            .filter_map(|l| l.trim().split_once('='))
            .find(|(a, _)| a.trim() == k)
            .map(|(_, b)| b.trim().to_string())
            .unwrap_or_else(|| panic!("conf/app.conf.test 缺键 {k}"))
    };
    let jwt = jwt_rust::Jwt::new(
        jwt_rust::config::JwtConfig {
            secret_key: val("secret"),
            issuer: "bee-rust-admin".into(),
            audience: "bee-rust-admin".into(),
            default_expire: 3600,
            ..Default::default()
        },
        std::sync::Arc::new(NoBlacklist),
    )
    .expect("测试侧 JWT 初始化");
    jwt.encode(&LegacyClaims { sub: admin_id, ver }, Some(3600)).expect("签老 token")
}

/// 设备列表（只列当前仍启用的设备），断言是数组。
async fn devices(c: &reqwest::Client, base: &str, token: &str) -> Vec<Value> {
    let (st, v) = call(c, Method::GET, format!("{base}/api/v1/auth/devices"), Some(token), None).await;
    assert_eq!(st, 200, "设备列表: {v}");
    v["data"].as_array().unwrap_or_else(|| panic!("设备列表必须是数组: {v}")).clone()
}

#[tokio::test]
async fn totp_bind_login_and_kick_devices() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let a = client("totp-dev-A");
    let b = client("totp-dev-B");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── 1. 未绑定时：普通登录照旧（不带 code 也能登），profile 明确报 false ──
    let (st, v) = login(&a, &base, "admin", "admin123", None).await;
    assert_eq!(st, 200, "未绑定时不该要验证码: {v}");
    let token_a = v["data"]["token"].as_str().unwrap().to_string();
    let (st, v) = profile(&a, &base, &token_a).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["user"]["totp_enabled"], false, "未绑定: {v}");

    // ── 2. 生成密钥：base32 32 字符（20 字节）+ otpauth URL + SVG 二维码 ──
    let (st, v) = call(&a, Method::POST, api("/auth/totp/setup"), Some(&token_a), None).await;
    assert_eq!(st, 200, "生成密钥: {v}");
    let secret = v["data"]["secret"].as_str().expect("secret 必须是字符串").to_string();
    assert_eq!(secret.len(), 32, "20 字节 → base32 32 字符（160 位，RFC 4226 §4 推荐）: {secret}");
    assert!(
        secret.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)),
        "base32（RFC 4648）字母表: {secret}"
    );
    let url = v["data"]["otpauth_url"].as_str().expect("otpauth_url 必须返回");
    assert!(url.starts_with("otpauth://totp/"), "{url}");
    assert!(url.contains(&format!("secret={secret}")), "{url}");
    assert!(url.contains("algorithm=SHA1"), "{url}");
    assert!(url.contains("digits=6"), "{url}");
    assert!(url.contains("period=30"), "{url}");
    let svg = v["data"]["qr_svg"].as_str().expect("qr_svg 必须返回");
    assert!(svg.contains("<svg xmlns") && svg.ends_with("</svg>"), "二维码是 SVG: {svg}");

    // 密钥落库必须是**密文**（项 7 硬口径①）：库里的值既不是明文，也不含明文子串
    let db_secret: String = sqlx::query_scalar("SELECT totp_secret FROM admin WHERE username = 'admin'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(db_secret, secret, "密钥不能明文落库");
    assert!(!db_secret.contains(&secret), "库里不能出现明文密钥: {db_secret}");
    assert!(db_secret.len() > secret.len(), "密文比明文长（带算法/随机 nonce）: {db_secret}");

    // 负控①：生成密钥这条审计不留摘要，且**任何审计行**都不含明文密钥
    let (st, v) = call(
        &a,
        Method::GET,
        api("/audit-logs?module=auth&size=50"),
        Some(&token_a),
        None,
    )
    .await;
    assert_eq!(st, 200, "{v}");
    let setup_row = v["data"]["list"]
        .as_array()
        .expect("list 是数组")
        .iter()
        .find(|r| r["action"] == "生成动态验证码密钥")
        .unwrap_or_else(|| panic!("审计里没有「生成动态验证码密钥」: {v}"))
        .clone();
    assert_eq!(setup_row["changes"], "", "密钥接口不产生变更摘要: {setup_row}");
    let leak: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE changes LIKE ? OR msg LIKE ? OR path LIKE ?",
    )
    .bind(format!("%{secret}%"))
    .bind(format!("%{secret}%"))
    .bind(format!("%{secret}%"))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leak, 0, "审计里不能出现明文密钥");

    // ── 3. 错误验证码绑定不上 ──
    let (st, v) = call(
        &a,
        Method::POST,
        api("/auth/totp/enable"),
        Some(&token_a),
        Some(json!({ "code": "000000" })),
    )
    .await;
    assert_eq!(st, 400, "错码不能绑定: {v}");
    assert_eq!(v["err"], "auth.bad_totp", "{v}");
    let (_, v) = profile(&a, &base, &token_a).await;
    assert_eq!(v["data"]["user"]["totp_enabled"], false, "错码之后仍是未绑定: {v}");

    // ── 4. 正确验证码绑定（测试侧按 RFC 算）──
    let (st, v) = call(
        &a,
        Method::POST,
        api("/auth/totp/enable"),
        Some(&token_a),
        Some(json!({ "code": code(&secret) })),
    )
    .await;
    assert_eq!(st, 200, "绑定: {v}");
    let (_, v) = profile(&a, &base, &token_a).await;
    assert_eq!(v["data"]["user"]["totp_enabled"], true, "绑定后 profile 报 true: {v}");

    // ── 5. 绑定后登录三态：不带 code 拒 / 错 code 拒 / 对 code 过 ──
    let (st, v) = login(&b, &base, "admin", "admin123", None).await;
    assert_eq!(st, 400, "绑定后不带验证码不能登录: {v}");
    assert_eq!(v["err"], "auth.totp_required", "{v}");
    let (st, v) = login(&b, &base, "admin", "admin123", Some("000000")).await;
    assert_eq!(st, 400, "错码不能登录: {v}");
    assert_eq!(v["err"], "auth.bad_totp", "{v}");
    let (st, v) = login(&b, &base, "admin", "admin123", Some(&code(&secret))).await;
    assert_eq!(st, 200, "对码登录: {v}");
    let token_b = v["data"]["token"].as_str().unwrap().to_string();

    // ── 5b. 缺码**不计**失败次数（钉住）：连来 6 次（> max_fail=5）也不该封禁，
    //     第 7 次带对码照样登得上。缺码是「协议没对齐」，不是有人在猜密码 ——
    //     计进去等于一个没升级的前端能把用户刷进封禁（本条是钉行为的测试，无代码改动）。
    for i in 0..6 {
        let (st, v) = login(&b, &base, "admin", "admin123", None).await;
        assert_eq!(st, 400, "第 {i} 次缺码应 400 而不是 429（缺码不计失败）: {v}");
        assert_eq!(v["err"], "auth.totp_required", "第 {i} 次缺码: {v}");
    }
    forget_step(&pool).await;
    let (st, v) = login(&b, &base, "admin", "admin123", Some(&code(&secret))).await;
    assert_eq!(st, 200, "6 次缺码之后对码仍能登录（缺码没把账号刷进封禁）: {v}");

    // ── 6. 设备列表：两台设备，当前设备唯一且标注出来 ──
    let list = devices(&b, &base, &token_b).await;
    assert_eq!(list.len(), 2, "两台设备（A / B 各自 UA）: {list:?}");
    let cur: Vec<&Value> = list.iter().filter(|d| d["current"] == true).collect();
    assert_eq!(cur.len(), 1, "当前设备只标一行: {list:?}");
    assert!(
        cur[0]["user_agent"].as_str().unwrap_or("").contains("totp-dev-B"),
        "当前设备是自己的 UA: {:?}",
        cur[0]
    );
    let dev_a = list
        .iter()
        .find(|d| d["user_agent"].as_str().unwrap_or("").contains("totp-dev-A"))
        .unwrap_or_else(|| panic!("列表里没有设备 A: {list:?}"));
    let dev_a_id = common::as_id(&dev_a["id"]);
    for d in &list {
        assert!(d["ip"].is_string(), "{d}");
        assert!(d["created_at"].is_string(), "{d}");
        assert!(d["last_active_at"].is_string(), "{d}");
    }

    // ── 7. 踢别人的设备 → 404（别人的行当不存在，不泄露 id 有效）──
    let (st, v) = call(
        &a,
        Method::POST,
        api("/admins"),
        Some(&token_a),
        Some(json!({ "username": "totpother", "password": "otherpass123", "nickname": "别人" })),
    )
    .await;
    assert_eq!(st, 200, "造第二个管理员: {v}");
    let c = client("totp-dev-C");
    let (st, v) = login(&c, &base, "totpother", "otherpass123", None).await;
    assert_eq!(st, 200, "第二个管理员登录（它没绑验证码）: {v}");
    let token_c = v["data"]["token"].as_str().unwrap().to_string();
    let (st, v) = call(
        &c,
        Method::DELETE,
        api(&format!("/auth/devices/{dev_a_id}")),
        Some(&token_c),
        None,
    )
    .await;
    assert_eq!(st, 404, "不能踢别人的设备: {v}");

    // ── 8. 踢掉 A：A 的下一次请求 401，B 不受影响（不是全局失效）──
    let (st, v) = call(
        &b,
        Method::DELETE,
        api(&format!("/auth/devices/{dev_a_id}")),
        Some(&token_b),
        None,
    )
    .await;
    assert_eq!(st, 200, "踢设备: {v}");
    let (st, v) = profile(&a, &base, &token_a).await;
    assert_eq!(st, 401, "被踢设备的旧 token 立刻失效: {v}");
    let (st, _) = profile(&b, &base, &token_b).await;
    assert_eq!(st, 200, "踢一台不影响另一台（不能拿 token_version 一刀切）");
    let list = devices(&b, &base, &token_b).await;
    assert_eq!(list.len(), 1, "被踢的不再出现在列表: {list:?}");

    // ── 9. 踢出 ≠ 封禁：同一 UA 再登录（带对码）能拿到新设备行 ──
    forget_step(&pool).await;
    let (st, v) = login(&a, &base, "admin", "admin123", Some(&code(&secret))).await;
    assert_eq!(st, 200, "被踢的设备重新登录: {v}");
    let token_a2 = v["data"]["token"].as_str().unwrap().to_string();
    let (st, _) = profile(&a, &base, &token_a2).await;
    assert_eq!(st, 200, "新 token 可用");
    let list = devices(&a, &base, &token_a2).await;
    assert_eq!(list.len(), 2, "重新登录生成新的设备行（旧行已停用）: {list:?}");
    // 只看 admin 自己的行（另一个管理员 totpother 也有一台设备，别把它算进来）
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_login_device WHERE status = 1 \
         AND admin_id = (SELECT id FROM admin WHERE username = 'admin')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(active, 2, "库里启用中的设备行与列表一致");

    // ── 10. 最后活跃时间会被刷新（隔久了才写一次）──
    let new_dev_id = common::dec_id(
        list.iter()
            .find(|d| d["user_agent"].as_str().unwrap_or("").contains("totp-dev-A"))
            .expect("新设备 A")["id"]
            .as_str()
            .unwrap(),
    );
    let stale = chrono::Local::now().naive_local() - chrono::Duration::hours(1);
    sqlx::query("UPDATE admin_login_device SET last_active_at = ? WHERE id = ?")
        .bind(stale)
        .bind(new_dev_id)
        .execute(&pool)
        .await
        .unwrap();
    let (st, _) = profile(&a, &base, &token_a2).await;
    assert_eq!(st, 200, "触发一次请求");
    let touched: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT last_active_at FROM admin_login_device WHERE id = ?")
            .bind(new_dev_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        touched > chrono::Local::now().naive_local() - chrono::Duration::minutes(5),
        "请求后 last_active_at 应回到当下，实际 {touched}"
    );

    // ── 11. 解绑：错码拒 / 对码过；解绑后不用验证码也能登录、密钥被清 ──
    let (st, v) = call(
        &a,
        Method::POST,
        api("/auth/totp/disable"),
        Some(&token_a2),
        Some(json!({ "code": "000000" })),
    )
    .await;
    assert_eq!(st, 400, "错码不能解绑: {v}");
    assert_eq!(v["err"], "auth.bad_totp", "{v}");
    let (st, v) = call(
        &a,
        Method::POST,
        api("/auth/totp/disable"),
        Some(&token_a2),
        Some(json!({ "code": code(&secret) })),
    )
    .await;
    assert_eq!(st, 200, "解绑: {v}");
    let (_, v) = profile(&a, &base, &token_a2).await;
    assert_eq!(v["data"]["user"]["totp_enabled"], false, "解绑后 profile 报 false: {v}");
    let db_secret: String = sqlx::query_scalar("SELECT totp_secret FROM admin WHERE username = 'admin'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(db_secret, "", "解绑要连密钥一起清掉（留着等于没解绑）");
    let (st, v) = login(&a, &base, "admin", "admin123", None).await;
    assert_eq!(st, 200, "解绑后不需要验证码: {v}");

    // ── 12. 老 token（升级前签的，payload 里没有 dv）走 logout-others ──
    // 换发的 token 必须挂到一台**新落的设备行**上：原样签回 dv=0 的话这台设备永远
    // 不出现在列表里（踢不掉），而且循环调用 logout-others 能一直续期。
    let (admin_id, ver): (i64, i32) =
        sqlx::query_as("SELECT id, token_version FROM admin WHERE username = 'admin'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let d = client("totp-dev-legacy");
    let legacy = legacy_token(admin_id, ver);
    let (st, v) = call(&d, Method::POST, api("/auth/logout-others"), Some(&legacy), None).await;
    assert_eq!(st, 200, "老 token 也要能换发: {v}");
    let token_l = v["data"]["token"].as_str().unwrap().to_string();
    let list = devices(&d, &base, &token_l).await;
    let legacy_dev = common::as_id(
        &list
            .iter()
            .find(|x| x["user_agent"].as_str().unwrap_or("").contains("totp-dev-legacy"))
            .unwrap_or_else(|| panic!("换发的 token 要挂在一台列表可见的设备行上: {list:?}"))["id"],
    );
    let (st, v) = call(
        &d,
        Method::DELETE,
        api(&format!("/auth/devices/{legacy_dev}")),
        Some(&token_l),
        None,
    )
    .await;
    assert_eq!(st, 200, "换发时挂上的设备要能踢掉: {v}");
    let (st, _) = profile(&d, &base, &token_l).await;
    assert_eq!(st, 401, "踢掉后换发的 token 立刻失效（dv=0 时永远做不到）");

    // 负控②：登录日志里也不能出现明文密钥（UA / 备注都会落这张表）
    let leak: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM login_log WHERE user_agent LIKE ? OR msg LIKE ?",
    )
    .bind(format!("%{secret}%"))
    .bind(format!("%{secret}%"))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leak, 0, "登录日志里不能出现明文密钥");
}
