// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 扩展工程 C3（项 6）：通知渠道（Webhook + SMTP）。起真实进程 + 真库（bee_admin_test），
//! 需要 `BEE_ADMIN_DB_DSN`。
//!
//! 假 webhook 与假 SMTP 都是这个文件里现起的**本机服务**：断言的是「服务端真的往外发了
//! 这个请求 / 这封信」，不是自己跟自己比。假 SMTP 只走明文（`smtp_encryption = none`）
//! —— 起一台带证书的 TLS 服务器不现实，加密传输本身是 lettre 自己的测试范围。
mod common;
#[path = "api_notify_test/helpers.rs"]
mod helpers;

use helpers::*;
use reqwest::Method;
use serde_json::{Value, json};

#[tokio::test]
async fn notify_webhook_events_and_smtp_test_mail() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let c = reqwest::Client::new();
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    let (hook, hook_port) = fake_webhook().await;
    let (smtp, smtp_port) = fake_smtp().await;
    let token = common::login(&base, "admin", "admin123").await.expect("登录失败");

    // ── 1. 默认配置：两个渠道都关着，口令没设置 ──────────────────
    let (st, v) = call(&c, Method::GET, api("/notify/config"), Some(&token), None).await;
    assert_eq!(st, 200, "通知配置: {v}");
    assert_eq!(v["data"]["webhook_enabled"], false, "{v}");
    assert_eq!(v["data"]["smtp_enabled"], false, "{v}");
    assert_eq!(v["data"]["smtp_password_set"], false, "默认没有口令: {v}");
    assert_eq!(v["data"]["smtp_port"], 587, "默认端口: {v}");

    // ── 2. 存配置（webhook + SMTP，口令非空）───────────────────
    let cfg = json!({
        "webhook_enabled": true,
        "webhook_url": format!("http://127.0.0.1:{hook_port}/hook"),
        "smtp_enabled": true,
        "smtp_host": "127.0.0.1",
        "smtp_port": smtp_port,
        "smtp_user": "notify-user@test.local",
        "smtp_password": SMTP_PW,
        "smtp_from": "bee-admin@test.local",
        "smtp_encryption": "none",
    });
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(cfg.clone())).await;
    assert_eq!(st, 200, "存通知配置: {v}");

    // 读回：字段在，口令**不在**（响应体里全文搜不到）
    let (st, raw) = call_raw(&c, Method::GET, api("/notify/config"), Some(&token), None).await;
    assert_eq!(st, 200);
    assert!(!raw.contains(SMTP_PW), "GET 配置不能回口令: {raw}");
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["data"]["smtp_password_set"], true, "口令已设置: {v}");
    assert_eq!(v["data"]["webhook_url"], format!("http://127.0.0.1:{hook_port}/hook"), "{v}");
    assert_eq!(v["data"]["smtp_port"], smtp_port, "{v}");
    assert_eq!(v["data"]["smtp_from"], "bee-admin@test.local", "{v}");

    // ── 3. 口令落库是密文、审计与日志里没有它 ──────────────────
    let stored: String = sqlx::query_scalar("SELECT value FROM sys_config WHERE config_key = 'notify.smtp_password'")
        .fetch_one(&pool)
        .await
        .expect("notify.smtp_password 键必须被种子写入");
    assert_ne!(stored, SMTP_PW, "落库不能是明文");
    assert!(!stored.contains(SMTP_PW), "落库不能含明文: {stored}");
    assert!(stored.len() > SMTP_PW.len(), "密文应长于明文: {stored}");
    // is_public 必须是 0：`/configs/public` 免登录，公开等于把内部地址全抖出去
    let public: i8 = sqlx::query_scalar("SELECT is_public FROM sys_config WHERE config_key = 'notify.smtp_password'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(public, 0, "通知配置一律私有");

    let leaked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE changes LIKE ? OR msg LIKE ? OR path LIKE ?",
    )
    .bind(format!("%{SMTP_PW}%"))
    .bind(format!("%{SMTP_PW}%"))
    .bind(format!("%{SMTP_PW}%"))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0, "审计里不能出现 SMTP 口令");
    // 变更摘要只记「改过」，不记值（密文列的口径，见 audit::Diff::masked）
    let changes: String = sqlx::query_scalar(
        "SELECT changes FROM audit_log WHERE path LIKE '%/notify/config' AND method = 'PUT' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let changes: Value = serde_json::from_str(&changes).unwrap();
    assert_eq!(changes["smtp_password"]["changed"], true, "摘要只记「已变更」: {changes}");
    assert_eq!(changes["smtp_password"]["old"], Value::Null, "不能记旧值: {changes}");
    assert_eq!(changes["webhook_url"]["new"], format!("http://127.0.0.1:{hook_port}/hook"), "{changes}");

    // ── 3b. 口令过长：报的是密文上限（255 字符），不甩「列宽」────────
    // 口令落在 `sys_config.value`（TEXT），卡的从来不是列宽 —— 报文必须说人话。
    let mut long = cfg.clone();
    long["smtp_password"] = json!("口".repeat(100)); // 100 个汉字 = 300 字节 → 密文 440 > 255
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(long)).await;
    assert_eq!(st, 400, "超长口令要 400: {v}");
    let msg = v["msg"].as_str().unwrap_or_default();
    assert!(!msg.contains("列宽"), "口令报错不能甩「列宽」（它落在 TEXT 列）: {msg}");
    assert!(msg.contains("255"), "要报出上限是多少: {msg}");

    // SMTP 关着也要拦长度（与 webhook 同类：值整条进审计摘要，超长 = 丢一条审计）；
    // 其余字段照抄 §2，万一没拦住（RED）也不让后续段落跟着塌
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(json!({
        "webhook_enabled": true,
        "webhook_url": format!("http://127.0.0.1:{hook_port}/hook"),
        "smtp_enabled": false,
        "smtp_host": "h".repeat(300),
        "smtp_port": smtp_port,
        "smtp_user": "notify-user@test.local",
        "smtp_from": "",
        "smtp_encryption": "none",
    }))).await;
    assert_eq!(st, 400, "SMTP 关着也要拦超长服务器地址: {v}");

    // ── 3c. Webhook IP 字面量：link-local / 未指定 → 400，回环放行 ──────
    // 接口层的接线验证（单测覆盖其余地址形态）：元数据地址是 SSRF 的典型目标。
    let mut meta = cfg.clone();
    meta["webhook_url"] = json!("http://169.254.169.254/latest/meta-data");
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(meta)).await;
    assert_eq!(st, 400, "webhook 指向 169.254.169.254 要 400: {v}");
    let msg = v["msg"].as_str().unwrap_or_default();
    assert!(msg.contains("链路本地"), "提示要说清拦的是什么: {msg}");

    let mut loopback = cfg.clone();
    loopback["webhook_url"] = json!("http://127.0.0.1:9/hook");
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(loopback)).await;
    assert_eq!(st, 200, "回环 127.0.0.1 保持放行: {v}");
    // 恢复成假 webhook 地址，§4 的事件照常收
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(cfg.clone())).await;
    assert_eq!(st, 200, "恢复假 webhook 地址: {v}");

    // ── 4. 公告发布 → webhook 收到事件体 ──────────────────────
    let (st, v) = publish(&c, &base, &token, "通知渠道验收公告", 1).await;
    assert_eq!(st, 200, "发公告: {v}");
    wait_until("webhook 首投", || hook.count() >= 1).await;
    settle().await;
    assert_eq!(hook.count(), 1, "一条公告一个事件，不多不少: {:?}", hook.bodies());
    let ev = &hook.bodies()[0];
    assert_eq!(ev["event"], "notice.published", "事件名: {ev}");
    assert_eq!(ev["data"]["title"], "通知渠道验收公告", "{ev}");
    assert!(ev["data"]["id"].as_str().is_some_and(|s| s.len() >= 8), "事件要带公告 id: {ev}");
    assert!(ev["at"].as_i64().is_some_and(|t| t > 1_700_000_000), "事件要带时间戳: {ev}");

    // 草稿不发（只有「发布」才是事件）
    let (st, _) = publish(&c, &base, &token, "草稿不该发", 0).await;
    assert_eq!(st, 200);
    settle().await;
    assert_eq!(hook.count(), 1, "草稿不发事件: {:?}", hook.bodies());

    // ── 5. 关掉渠道 → 不发（负控：不是「凡是写公告都打」）─────────
    let mut off = cfg.clone();
    off["webhook_enabled"] = json!(false);
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(off.clone())).await;
    assert_eq!(st, 200, "{v}");
    let (st, _) = publish(&c, &base, &token, "关掉后不发", 1).await;
    assert_eq!(st, 200);
    settle().await;
    assert_eq!(hook.count(), 1, "渠道关着就不该发: {:?}", hook.bodies());

    // ── 6. 首发失败落库待重投：请求里只试一次（重投归 notify_retry，另一个用例覆盖）──
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(cfg.clone())).await;
    assert_eq!(st, 200, "{v}");
    hook.fail_first(1);
    let before = hook.count();
    let (st, _) = publish(&c, &base, &token, "失败落库待重投", 1).await;
    assert_eq!(st, 200);
    wait_until("首发失败", || hook.count() >= before + 1).await;
    settle().await;
    assert_eq!(hook.count(), before + 1, "首发只尝试一次，不在请求内重试: {:?}", hook.bodies());
    let (status, attempts, msg, created_at, next_at, payload): (
        String,
        i32,
        String,
        chrono::NaiveDateTime,
        Option<chrono::NaiveDateTime>,
        String,
    ) = sqlx::query_as(
        "SELECT status, attempts, msg, created_at, next_at, payload \
         FROM notify_delivery ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("失败首发必须落 notify_delivery");
    assert_eq!(status, "pending", "普通失败进 pending");
    assert_eq!(attempts, 1, "首发算一次尝试");
    assert!(msg.contains("HTTP 500"), "原因要留 500: {msg}");
    assert!(payload.contains("失败落库待重投"), "payload 是实际发出的原文: {payload}");
    assert_eq!(
        next_at.expect("pending 必须有 next_at").signed_duration_since(created_at).num_seconds(),
        60,
        "首发失败 1 分钟后重投"
    );

    // ── 7. 成功不落行：账本行数不变 ─────────────────────────────
    hook.fail_first(0);
    let rows_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notify_delivery")
        .fetch_one(&pool)
        .await
        .unwrap();
    let before = hook.count();
    let (st, _) = publish(&c, &base, &token, "成功后不落账本", 1).await;
    assert_eq!(st, 200);
    wait_until("成功后投递", || hook.count() >= before + 1).await;
    settle().await;
    assert_eq!(hook.count(), before + 1, "成功只发一次: {:?}", hook.bodies());
    let rows_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notify_delivery")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows_after, rows_before, "成功不写 notify_delivery");

    // ── 8. 账号锁定 → account.locked 事件 ─────────────────────
    hook.fail_first(0);
    // 这 5 次失败会把「账号」与「IP」两个桶一起刷封，所以单开一个 IP：
    // 共用 IP 桶会把后面所有登录（包括别的用例）一起封掉。
    let attacker = reqwest::Client::builder()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("x-real-ip", "10.9.9.9".parse().unwrap());
            h
        })
        .build()
        .unwrap();
    for _ in 0..5 {
        let (st, v) = call(
            &attacker,
            Method::POST,
            api("/auth/login"),
            None,
            Some(json!({ "username": "notifylock", "password": "wrong-pass" })),
        )
        .await;
        assert_ne!(st, 200, "错密码不该成功: {v}");
    }
    wait_until("账号锁定事件", || {
        hook.bodies().iter().any(|e| e["event"] == "account.locked")
    })
    .await;
    settle().await;
    let locked: Vec<Value> =
        hook.bodies().into_iter().filter(|e| e["event"] == "account.locked").collect();
    assert_eq!(locked.len(), 1, "锁一次只发一个事件: {locked:?}");
    assert_eq!(locked[0]["data"]["username"], "notifylock", "{:?}", locked[0]);
    assert!(locked[0]["at"].as_i64().is_some_and(|t| t > 1_700_000_000), "{:?}", locked[0]);

    // ── 9. 测试邮件真的发到当前登录者的邮箱（密文列解密后使用）──
    let (st, v) = call(
        &c,
        Method::PUT,
        api("/auth/profile"),
        Some(&token),
        Some(json!({ "nickname": "超管", "email": "notify-owner@test.local", "phone": "" })),
    )
    .await;
    assert_eq!(st, 200, "改自己邮箱: {v}");

    let (st, v) = call(&c, Method::POST, api("/notify/test-email"), Some(&token), None).await;
    assert_eq!(st, 200, "测试邮件应成功: {v}");
    let rcpt = smtp
        .lines()
        .into_iter()
        .find(|l| l.to_uppercase().starts_with("RCPT TO"))
        .expect("假 SMTP 必须收到 RCPT TO");
    assert!(rcpt.contains("notify-owner@test.local"), "收件人应是当前登录者的邮箱: {rcpt}");
    assert!(
        auth_has_password(&smtp.lines()),
        "SMTP 认证要用解出来的口令（库里存的是密文）: {:?}",
        smtp.lines()
    );

    // ── 10. 失败要给原因，且原因里不含口令 ─────────────────────
    let mut bad = cfg.clone();
    bad["smtp_port"] = json!(1); // 127.0.0.1:1 必然连不上
    let (st, v) = call(&c, Method::PUT, api("/notify/config"), Some(&token), Some(bad)).await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = call(&c, Method::POST, api("/notify/test-email"), Some(&token), None).await;
    assert_eq!(st, 400, "连不上要报失败: {v}");
    let msg = v["msg"].as_str().unwrap_or_default();
    assert!(msg.contains("失败"), "失败原因要说明白: {v}");
    assert!(!msg.contains(SMTP_PW), "失败原因里不许回口令: {v}");

    // ── 11. 通用配置页看不见也改不动这个口令键 ─────────────────
    let (st, v) = call(&c, Method::GET, api("/configs"), Some(&token), None).await;
    assert_eq!(st, 200, "{v}");
    let keys: Vec<&str> =
        v["data"]["list"].as_array().unwrap().iter().filter_map(|r| r["key"].as_str()).collect();
    assert!(!keys.contains(&"notify.smtp_password"), "通用配置页不列口令键: {keys:?}");
    assert!(
        !keys.iter().any(|k| k.starts_with("notify.")),
        "通用配置页不列任何 notify.*（整组由通知设置页维护）: {keys:?}"
    );

    let pw_id: i64 = sqlx::query_scalar("SELECT id FROM sys_config WHERE config_key = 'notify.smtp_password'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let pw_hid = common::enc_id(pw_id as u64);
    let (st, v) = call(
        &c,
        Method::PUT,
        format!("{base}/api/v1/configs/{pw_hid}"),
        Some(&token),
        Some(json!({ "value": "plaintext-pw" })),
    )
    .await;
    assert_eq!(st, 400, "通用配置页改口令要拒（否则「加密存」形同虚设）: {v}");
    let (st, _) = call(&c, Method::DELETE, format!("{base}/api/v1/configs/{pw_hid}"), Some(&token), None).await;
    assert_eq!(st, 400, "通用配置页删口令也要拒");
    // 被拒的那次不能把明文写进库。注意**不能**跟前面那次密文比字节：AEAD 是随机 nonce，
    // 同一条口令重存一次就是另一个串（上面第 10 步改端口时重存过），比的是「还是密文」。
    let after: String = sqlx::query_scalar("SELECT value FROM sys_config WHERE config_key = 'notify.smtp_password'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(after, "plaintext-pw", "被拒的改写不能落库");
    assert!(!after.contains(SMTP_PW), "仍是密文: {after}");
    assert_eq!(after.len(), stored.len(), "同一明文重加密长度不变: {after} vs {stored}");

    // 权限码：普通用户看不到通知设置（超管是 *:*:*，这里只钉权限码本身存在且能配）
    let (st, v) = call(&c, Method::GET, api("/menus/tree"), Some(&token), None).await;
    assert_eq!(st, 200, "{v}");
    let mut perms = Vec::new();
    collect_perms(&v["data"], &mut perms);
    assert!(perms.contains(&"system:notify:list".to_string()), "菜单树要有通知设置: {perms:?}");
    assert!(perms.contains(&"system:notify:edit".to_string()), "{perms:?}");
}
