// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! F4-2 / F4-3 / F4-4：Webhook HMAC 签名、失败落库重投、发送时 DNS 封禁。
//! 起真实进程 + 真库（bee_admin_test），需要 `BEE_ADMIN_DB_DSN`。
//!
//! 假 webhook 收**原始字节**与头：签名必须由测试侧独立 HMAC 复算，不能只看
//! 「头存在」。重投通过 `POST /api/v1/jobs/{id}/run` 手动触发（接口是异步的，
//! 用 `job_log` 判定这一趟跑完）。
mod common;
#[path = "api_webhook_retry_test/helpers.rs"]
mod helpers;

use axum::http::StatusCode;
use helpers::*;
use reqwest::Method;
use serde_json::Value;
use std::time::Duration;

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

    // 恢复密钥，供后面的重投用例用（重投也是「每投现签」，同一个密钥、新的时间戳）
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

    // ── 8. 领取竞争（M1）：同一行被两个领取者抢，只有一个发出 ──
    // 先恢复 webhook 地址（第 7 节改成了 169.254 字面量，不恢复就一个请求都收不到）。
    let (st, _) = save_cfg(&c, &base, &token, cfg_json(hook_port, Some(SECRET))).await;
    assert_eq!(st, 200);
    // 造两行 due 记录，A 在前。假 webhook 慢 2 秒：等 A 的请求到达后，测试侧把 B 的
    // next_at 推到未来（模拟另一个领取者已经抢走 B）。本趟 CAS 用的是 SELECT 时的
    // 旧 next_at，对 B 必然影响 0 行 → 跳过、不发出、不覆盖 B 的排期。
    let ra = common::new_row_id();
    let rb = common::new_row_id();
    let past_a = chrono::Utc::now().naive_utc() - chrono::Duration::seconds(20);
    let past_b = chrono::Utc::now().naive_utc() - chrono::Duration::seconds(10);
    for (id, title, at) in [(ra, "领取竞争A", past_a), (rb, "领取竞争B", past_b)] {
        sqlx::query(
            "INSERT INTO notify_delivery (id, event, payload, status, attempts, next_at, msg, created_at) \
             VALUES (?, 'notice.published', ?, 'pending', 1, ?, '手工造', ?)",
        )
        .bind(id)
        .bind(format!(
            "{{\"event\":\"notice.published\",\"at\":1700000000,\"data\":{{\"title\":\"{title}\"}}}}"
        ))
        .bind(at)
        .bind(at)
        .execute(&pool)
        .await
        .unwrap();
    }
    hook.delay_ms(2000);
    let before = hook.count();
    let job_id: i64 = sqlx::query_scalar("SELECT id FROM job WHERE code = 'notify_retry'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let last = common::last_log_id(&pool, "notify_retry").await;
    common::trigger(&c, &base, &common::enc_id(job_id as u64), &token).await;
    wait_until("A 先到达", || hook.count() >= before + 1).await;
    let other_claim = (chrono::Local::now() + chrono::Duration::minutes(10)).naive_local();
    sqlx::query("UPDATE notify_delivery SET next_at = ? WHERE id = ?")
        .bind(other_claim)
        .bind(rb)
        .execute(&pool)
        .await
        .unwrap();
    let (status, msg, _) = common::wait_job_log(&pool, "notify_retry", last).await;
    assert_eq!(status, 1, "notify_retry 应成功: {msg}");
    assert!(
        msg.contains("重投 1 条") && msg.contains("跳过 1 条"),
        "这一趟只能发 A、跳过 B: {msg}"
    );
    assert_eq!(hook.count(), before + 1, "B 不能再发一次: {}", hook.count());
    let (_, st_a, at_a, _, _) = delivery(&pool, "领取竞争A").await;
    assert_eq!((st_a.as_str(), at_a), ("delivered", 2), "A 应正常投递");
    let (_, st_b, at_b, next_b, _) = delivery(&pool, "领取竞争B").await;
    assert_eq!((st_b.as_str(), at_b), ("pending", 1), "B 被跳过，尝试次数不动");
    assert_eq!(
        next_b.map(|t| t.and_utc().timestamp()),
        Some(other_claim.and_utc().timestamp()),
        "B 的 next_at 不能被本趟覆盖"
    );
    hook.delay_ms(0);

    // ── 9. 不跟随重定向（I1）：302/307 按普通失败入账，绝不跟到链路本地目标 ──
    for (code, label) in [
        (StatusCode::FOUND, "重定向302验收"),
        (StatusCode::TEMPORARY_REDIRECT, "重定向307验收"),
    ] {
        hook.redirect(code, "http://169.254.169.254/latest/meta-data");
        let before = hook.count();
        publish(&c, &base, &token, label).await;
        wait_until(label, || hook.count() >= before + 1).await;
        wait_row(&pool, label).await;
        let (_, status, attempts, next_at, err) = delivery(&pool, label).await;
        assert_eq!(status, "pending", "3xx 是普通失败，进重试: {err}");
        assert_eq!(attempts, 1, "首发算一次");
        assert!(next_at.is_some(), "普通失败要有重投排期");
        assert_eq!(err, format!("HTTP {}", code.as_u16()), "要如实记 3xx 状态码");
        assert_eq!(
            hook.count(),
            before + 1,
            "只能有 1 个请求（跟随重定向会多打一次目标地址）"
        );
        hook.clear_redirect();
    }

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
