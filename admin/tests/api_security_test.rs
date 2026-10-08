// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 阶段 5a：请求安全扫描（security-rust 3.0.0）的后端契约。
//! 契约见 `docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md`「阶段 5a 契约 · 请求安全扫描」。
//!
//! 需要 BEE_ADMIN_DB_DSN（真库，库名以 `_test` 结尾）。`scan = high` 的用例**另起一个进程**
//! （阈值在启动时读，配置非法还会拒绝启动），两个进程共用同一个库 —— 断言按「路径 + 行序」区分。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

/// 查询串里的 SQLi（**已 URL 编码**）：空格型载荷，原文直接认得出。
const SQLI_QUERY: &str = "1%20UNION%20SELECT%20password%20FROM%20admin";

/// 编码掉引号的 SQLi（`1' OR '1'='1-- ` 的编码形态）：原文 0 命中（E2E 实测），
/// 靠 `decode_url` 解码后再扫才认得出 —— 真实攻击就是这个形态。
const SQLI_QUERY_ENCODED: &str = "1%27%20OR%20%271%27%3D%271--%20";

/// 体内的 SQLi（JSON 体不经 URL 编码，经典形态能整条送到）。
const SQLI_BODY: &str = "1' OR '1'='1";

/// 头里的 XSS（事件处理器形态，插件判 Critical）。
const XSS_UA: &str = "<img src=x onerror=alert(1)>";

/// 统一请求：返回 (状态码, JSON body)。
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

/// 扫描留痕的行（按 id 升序）：`(status, msg)`。
async fn scan_rows(pool: &sqlx::MySqlPool, path: &str) -> Vec<(i8, String)> {
    sqlx::query_as(
        "SELECT status, msg FROM audit_log WHERE action = '安全扫描命中' AND path = ? ORDER BY id",
    )
    .bind(path)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// 该路径上「非扫描」的审计行数（= `audit_mw` 写的那种），用来证明中间件的层级顺序。
async fn audit_rows(pool: &sqlx::MySqlPool, path: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE action <> '安全扫描命中' AND path = ?")
        .bind(path)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn notice_count(pool: &sqlx::MySqlPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM notice").fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn security_scan_reports_then_blocks() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let c = reqwest::Client::new();

    // ══ 一、默认 `[security] scan = off`：只报告，响应完全不受影响 ═══════════
    let (_off, base) = common::start_server().await;

    // 查询串里的 SQLi：health 不鉴权不查库，200 + 原文 "OK" 就是「响应不受影响」的证据
    let r = c
        .get(format!("{base}/api/v1/health?q={SQLI_QUERY}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200, "off 模式必须照常放行");
    assert_eq!(r.text().await.unwrap(), "OK", "放行时响应体不变");

    let rows = scan_rows(&pool, "/api/v1/health").await;
    assert_eq!(rows.len(), 1, "命中要留痕一条: {rows:?}");
    let (status, msg) = &rows[0];
    assert_eq!(*status, 1, "off 模式请求照常走完 → status=1（成功）");
    assert!(msg.contains("sql_injection"), "msg 要记命中的检测器名: {msg}");
    assert!(msg.contains("CRITICAL"), "msg 要记等级: {msg}");
    assert!(
        !msg.contains("UNION") && !msg.contains("password"),
        "原始 payload 绝不能进审计日志（长期保留的库不存攻击串）: {msg}"
    );

    // 百分号编码的 SQLi（引号被编成 %27，原文认不出）：解码后再扫，off 模式同样放行 + 留痕
    let r = c
        .get(format!("{base}/api/v1/health?q={SQLI_QUERY_ENCODED}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200, "off 模式照常放行");
    let rows = scan_rows(&pool, "/api/v1/health").await;
    assert_eq!(rows.len(), 2, "编码态的命中也要留痕: {rows:?}");
    assert!(rows[1].1.contains("sql_injection"), "解码后要认出 SQLi: {}", rows[1].1);

    // JSON 体里的 SQLi：**扫完原样放回**，业务照常解析 —— 登录能成功就是证明
    // （body 被吃掉的话 Json 提取器会回 400「请求体格式错误」）。附带字段只是载体。
    let (st, v) = call(
        &c,
        Method::POST,
        format!("{base}/api/v1/auth/login"),
        None,
        Some(json!({"username": "admin", "password": "admin123", "remark": SQLI_BODY})),
    )
    .await;
    assert_eq!(st, 200, "off 模式登录照常（体扫完要放回去）: {v}");
    let admin = v["data"]["token"].as_str().expect("登录 token").to_string();
    let rows = scan_rows(&pool, "/api/v1/auth/login").await;
    assert_eq!(rows.len(), 1, "体里的命中也要留痕: {rows:?}");
    assert!(rows[0].1.contains("sql_injection"), "{}", rows[0].1);

    // 头里的命中（UA）：也扫、也留痕、也放行
    let r = c
        .get(format!("{base}/api/v1/health"))
        .header("User-Agent", XSS_UA)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200, "off 模式带头部命中照常放行");
    assert_eq!(scan_rows(&pool, "/api/v1/health").await.len(), 3, "头部命中是新的一条");

    // 误报护栏（下限）：最普通的查询串不留痕
    let (st, v) = call(
        &c,
        Method::GET,
        format!("{base}/api/v1/admins?page=1&size=10&username=admin"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert!(scan_rows(&pool, "/api/v1/admins").await.is_empty(), "正常查询串不该留痕");

    // ══ 二、`[security] scan = high`：到阈值 403 + err = security.blocked ═════
    let (_hi, base) = common::start_server_conf(|conf| format!("{conf}\n[security]\nscan = high\n")).await;
    let admin = common::login(&base, "admin", "admin123").await.expect("high 模式登录");

    // 查询串命中 → 拦：信封形状与 C3 契约一致（静态码，不带 args）
    let (st, v) = call(&c, Method::GET, format!("{base}/api/v1/health?q={SQLI_QUERY}"), None, None).await;
    assert_eq!(st, 403, "达到阈值的命中要拦住: {v}");
    assert_eq!(v["code"], 403, "{v}");
    assert_eq!(v["err"], "security.blocked", "{v}");
    assert_eq!(v["msg"], "请求被安全策略拦截", "msg 是中文原文: {v}");
    assert!(v.get("args").is_none(), "静态码整个不带 args 键: {v}");
    let rows = scan_rows(&pool, "/api/v1/health").await;
    assert_eq!(rows.last().unwrap().0, 0, "被拦的请求 status=0（失败）: {rows:?}");

    // 百分号编码的 SQLi（原文认不出、解码后才命中）：high 档下同样要拦
    let (st, v) = call(
        &c,
        Method::GET,
        format!("{base}/api/v1/health?q={SQLI_QUERY_ENCODED}"),
        None,
        None,
    )
    .await;
    assert_eq!(st, 403, "编码后的 SQLi 也要拦住（解码后扫）: {v}");
    assert_eq!(v["err"], "security.blocked", "{v}");
    assert_eq!(scan_rows(&pool, "/api/v1/health").await.last().unwrap().0, 0, "同样留痕 status=0");

    // Referer 也是 URL：查询串里编码掉的载荷同理要拦（这里带的是已解码命中的那条）
    let r = c
        .get(format!("{base}/api/v1/health"))
        .header("Referer", format!("http://127.0.0.1/nothing?q={SQLI_QUERY_ENCODED}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403, "Referer 查询串里的编码 SQLi 要拦");

    // 体命中 → 拦，且**副作用没有发生**（公告没建出来）
    let (st, v) = call(
        &c,
        Method::POST,
        format!("{base}/api/v1/notices"),
        Some(&admin),
        Some(json!({"title": "扫到了", "content": SQLI_BODY, "status": 1})),
    )
    .await;
    assert_eq!(st, 403, "体里的命中要拦住: {v}");
    assert_eq!(v["err"], "security.blocked", "{v}");
    assert_eq!(notice_count(&pool).await, 0, "被拦的请求不能产生副作用");

    // 顺序证据：安全扫描在**审计层之外**，被拦的请求根本走不到审计层 ——
    // POST 是审计层会记的操作，这里没有「新增公告」的行就是顺序对了
    assert_eq!(audit_rows(&pool, "/api/v1/notices").await, 0, "被拦的请求不进审计层");

    // 误报护栏（拦截模式下的底线）：正常请求一律要能走完
    // （这条路径上已经有一条被拦时的扫描留痕，比较条数：正常请求不该再多出一条）
    let notices_scans = scan_rows(&pool, "/api/v1/notices").await.len();
    let (st, v) = call(
        &c,
        Method::POST,
        format!("{base}/api/v1/notices"),
        Some(&admin),
        Some(json!({
            "title": "系统维护通知",
            "content": "今晚 22:00-23:00 例行维护，期间可能短暂不可用；折扣 50% off（限时 = 3 天）。",
            "status": 1
        })),
    )
    .await;
    assert_eq!(st, 200, "正常中文公告不能被拦: {v}");
    assert_eq!(notice_count(&pool).await, 1);
    assert_eq!(audit_rows(&pool, "/api/v1/notices").await, 1, "放行的写操作照常审计");
    assert_eq!(
        scan_rows(&pool, "/api/v1/notices").await.len(),
        notices_scans,
        "正常公告不该新增扫描痕"
    );

    // 误报护栏：正常 GET（带查询串 + 中文查询值）也不能被拦
    let (st, v) = call(
        &c,
        Method::GET,
        format!("{base}/api/v1/admins?page=1&size=10"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(st, 200, "正常列表查询不能被拦: {v}");

    // multipart（头像分片上传）：体**不读不扫**，上传路径照常。
    // 表单里额外塞一个带攻击串的字段：它要是被扫了就会 403；这里 200（error=0）就是
    // 「multipart 一个字节都没读」的证据 —— 读走了的话 preprocess 解析不到表单字段。
    let bytes = b"security-scan-multipart-probe".to_vec();
    let hash = aetherupload::md5_hex(&bytes);
    let form = reqwest::multipart::Form::new()
        .text("resource_name", "probe.png")
        .text("resource_size", bytes.len().to_string())
        .text("resource_hash", hash)
        .text("group", "avatar")
        .text("locale", "zh")
        .text("note", SQLI_BODY);
    let (st, v) = {
        let r = c
            .post(format!("{base}/api/v1/avatar/upload/preprocess"))
            .bearer_auth(&admin)
            .multipart(form)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    };
    assert_eq!(st, 200, "multipart 上传不能被拦（体不扫）: {v}");
    assert_eq!(v["data"]["error"], 0, "上传预处理照常成功: {v}");
    assert!(
        scan_rows(&pool, "/api/v1/avatar/upload/preprocess").await.is_empty(),
        "multipart 的体不扫，不该留扫描痕"
    );
}
