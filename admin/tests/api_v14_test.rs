// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! BRD v1.4 后端新能力集成测试：A1 路由级 404/405 信封 / A2 登录时序侧信道（行为面）/
//! A3 退出其他设备 / A4 日志保留策略 / B6 流式 CSV 导出。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

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

/// 取 CSV 响应体（含 BOM），校验附件头，返回去掉 BOM 的文本行。
async fn csv_lines(c: &reqwest::Client, url: String, token: &str) -> Vec<String> {
    let r = c.get(url).bearer_auth(token).send().await.expect("导出请求失败");
    assert_eq!(r.status(), 200, "导出必须 200");
    assert_eq!(r.headers()["content-type"], "text/csv; charset=utf-8", "导出 Content-Type");
    let cd = r.headers()["content-disposition"].to_str().unwrap().to_string();
    assert!(cd.contains("attachment") && cd.contains(".csv"), "附件头: {cd}");
    let body = r.bytes().await.unwrap().to_vec();
    assert_eq!(&body[..3], &[0xEF, 0xBB, 0xBF], "首字节必须是 UTF-8 BOM");
    String::from_utf8(body[3..].to_vec())
        .expect("BOM 之后是 UTF-8")
        .trim_end()
        .split("\r\n")
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn v14_backend_features() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;

    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── A1 路由级 404/405 也走统一信封 ──────────────────────
    let r = c.get(api("/nope/none")).send().await.unwrap();
    assert_eq!(r.status(), 404);
    let ct = r.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ct.starts_with("application/json"), "未匹配路径必须是 JSON 信封，实际 {ct}");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["code"], 404, "{v}");
    assert_eq!(v["msg"], "接口不存在", "{v}");
    assert!(v["data"].is_null(), "信封形状一致: {v}");

    // /api/v1/health 只注册了 GET
    let r = c.post(api("/health")).send().await.unwrap();
    assert_eq!(r.status(), 405);
    assert_eq!(r.headers()["content-type"], "application/json", "405 也必须是信封");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["code"], 405, "{v}");
    assert_eq!(v["msg"], "方法不允许", "{v}");

    // 树外的路径不受兜底影响（保持 axum 原样：空 body、无 content-type，不替别的服务改响应形状）
    let r = c.get(format!("{base}/nope")).send().await.unwrap();
    assert_eq!(r.status(), 404);
    let ct = r
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    assert!(!ct.contains("json"), "树外保持 axum 原样，实际 {ct}");
    assert!(r.bytes().await.unwrap().is_empty(), "树外 404 是空 body，不是信封");

    // 未匹配路径上的写操作仍被审计留痕（兜底挂在审计中间件之内）
    let (st, _v) = call(&c, Method::POST, api("/nope/none"), None, Some(json!({}))).await;
    assert_eq!(st, 404, "未知路径的 POST 也回信封");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE path = '/api/v1/nope/none'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "探测性写操作也要留痕: {n}");

    // ── A2 用户不存在：提示与失败记录不变（时序本身集成测试断言不了）──
    let (st, v) = call(&c, Method::POST, api("/auth/login"), None,
        Some(json!({"username": "ghost_none", "password": "whatever123"}))).await;
    assert_eq!(st, 400, "{v}");
    assert_eq!(v["msg"], "用户名或密码错误", "与密码错误同一句提示，不泄露用户是否存在: {v}");
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM login_log WHERE username = 'ghost_none' AND msg = '用户不存在'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "不存在的用户也要留失败记录: {n}");

    // ── A3 退出其他设备 ────────────────────────────────────
    // 两次登录 = 两台设备（JWT 到秒，同一秒内两枚 token 可能一模一样，
    // 所以断言只看有效性，不比 token 串是否相同）
    let t1 = common::login(&base, "admin", "admin123").await.expect("设备 1 登录");
    let t2 = common::login(&base, "admin", "admin123").await.expect("设备 2 登录");
    let (st, v) = call(&c, Method::POST, api("/auth/logout-others"), Some(&t1), None).await;
    assert_eq!(st, 200, "退出其他设备: {v}");
    let fresh = v["data"]["token"].as_str().expect("必须回新 token（前端从 data.token 取）").to_string();
    assert!(v["data"]["expires_in"].as_i64().unwrap_or(0) > 0, "与 login 同形状: {v}");

    let (st, v) = call(&c, Method::GET, api("/auth/profile"), Some(&fresh), None).await;
    assert_eq!(st, 200, "当前设备换发的新 token 立即可用: {v}");
    assert_eq!(v["data"]["user"]["username"], "admin");

    let (st, _v) = call(&c, Method::GET, api("/auth/profile"), Some(&t2), None).await;
    assert_eq!(st, 401, "其他设备的 token 被踢下线");
    let (st, _v) = call(&c, Method::GET, api("/auth/profile"), Some(&t1), None).await;
    assert_eq!(st, 401, "本机旧 token 同样失效（token_version 已自增）");

    let admin = fresh; // 后面继续用当前设备换发的新 token

    // ── B6 流式导出：跨批不丢不重 ──────────────────────────
    // 塞 5001 行（> csv::BATCH = 5000）逼出第二批；导出条数必须与库里行数严格相等
    let mut values = Vec::with_capacity(5001);
    for i in 0..5001 {
        values.push(format!("(1,'bulk{i}','10.0.0.1','',1,'',NOW())"));
    }
    for chunk in values.chunks(500) {
        sqlx::query(&format!(
            "INSERT INTO login_log (admin_id, username, ip, user_agent, status, msg, created_at) VALUES {}",
            chunk.join(",")
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_log").fetch_one(&pool).await.unwrap();
    // id 列是 BIGINT UNSIGNED，sqlx 只肯解码成 u64（COUNT(*) 才有符号）
    let (min_id, max_id): (u64, u64) =
        sqlx::query_as("SELECT MIN(id), MAX(id) FROM login_log").fetch_one(&pool).await.unwrap();

    let lines = csv_lines(&c, api("/login-logs/export"), &admin).await;
    assert_eq!(lines[0], "ID,用户名,IP,User-Agent,结果,详情,时间", "表头");
    assert_eq!(lines.len() as i64, total + 1, "表头 + 全部 {total} 行，跨批不丢不重");
    assert!(lines[1].starts_with(&format!("{max_id},")), "id DESC：首行是最新的 {max_id}: {}", lines[1]);
    assert!(
        lines[lines.len() - 1].starts_with(&format!("{min_id},")),
        "最老的一行在（最后一批没被漏掉）: {}",
        lines[lines.len() - 1]
    );

    // 筛选与 list 同源（同一个 filtered()）：导出带上 username 过滤，条数跟着变
    let lines = csv_lines(&c, api("/login-logs/export?username=bulk0"), &admin).await;
    assert_eq!(lines.len(), 2, "username=bulk0 只命中一行: {lines:?}");

    // 另两个导出同样走流式路径，附件头/表头不变
    for (path, head) in [("/admins/export", "ID,用户名,昵称"), ("/audit-logs/export", "ID,时间,用户")] {
        let lines = csv_lines(&c, api(path), &admin).await;
        assert!(lines[0].starts_with(head), "{path} 表头: {}", lines[0]);
    }

    // ── A4 日志保留策略（[log] retain_days 默认 90）──────────
    // 各插一条过期行（2000 年）；清理在进程启动时跑，重启一个进程来触发（不真等 24 小时）
    sqlx::query(
        "INSERT INTO login_log (admin_id, username, ip, user_agent, status, msg, created_at) \
         VALUES (1, 'old-login', '1.1.1.1', '', 1, '', '2000-01-01 00:00:00')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO audit_log (admin_id, username, module, action, method, path, status, msg, duration_ms, ip, created_at) \
         VALUES (1, 'admin', 'other', '老记录', 'POST', '/api/v1/x', 0, '', 1, '1.1.1.1', '2000-01-01 00:00:00')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (_server2, base2) = common::start_server().await;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let old_login: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM login_log WHERE created_at < '2000-01-02'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let old_audit: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE created_at < '2000-01-02'")
                .fetch_one(&pool)
                .await
                .unwrap();
        if old_login == 0 && old_audit == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "启动清理没在 20 秒内删掉过期日志（login_log {old_login} / audit_log {old_audit}）"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // 保留窗口内的行一条都不能少（上面刚塞的 5001 行还在）
    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_log").fetch_one(&pool).await.unwrap();
    assert_eq!(kept, total, "窗口内的日志必须原样保留: {kept} != {total}");

    // 触发清理的那个进程也要是能用的
    let r = c.get(format!("{base2}/api/v1/health")).send().await.unwrap();
    assert_eq!(r.status(), 200);
}
