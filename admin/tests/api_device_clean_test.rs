// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 登录设备清理任务（批次 E · E2-2）的契约：只清「**已踢**且久未露面」的行。
//!
//! 三个样本刻意各钉住一条边界：
//! - **老软删**（`status=0`、40 天没露面、但 `created_at` 是昨天）→ 该清。
//!   `created_at` 故意造成「新」的：清掉它才能证明口径看的是 `last_active_at`，
//!   而不是分派说明里写的 `updated_at`（这张表**没有**这一列，见 models/login_device.rs）。
//! - **新软删**（昨天刚踢、`created_at` 是 100 天前）→ 留着（窗口是 30 天，且同样证明不看 created_at）。
//! - **在用行**（`status=1`、也是 40 天没露面）→ 一律不碰：在用的设备行是登录凭据的一部分
//!   （token 里带着它的 id），删了等于把在线的人踢下线。
//! 最后再跑一次钉幂等：没有可清的，计数归 0，任务仍然成功。
mod common;

use common::{dec_id, dsn, last_log_id, login, my_id, new_row_id, reset_db, start_server_conf, trigger, wait_job_log};
use reqwest::{Client, Method};
use serde_json::Value;
use sqlx::MySqlPool;

/// 任务在测试库里的默认窗口（`[auth] device_keep_days` 的默认值，conf/app.conf.test 没写它）。
/// 断言里写死这个数：它同时也是**文档口径**，改了要一起动 usage / app.conf.example。
const KEEP_DAYS: i64 = 30;

async fn call(c: &Client, method: Method, url: String, token: &str) -> (u16, Value) {
    let r = c.request(method, url).bearer_auth(token).send().await.expect("请求发送失败");
    (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
}

async fn job_id(c: &Client, base: &str, token: &str, code: &str) -> String {
    let (st, v) = call(c, Method::GET, format!("{base}/api/v1/jobs?size=100"), token).await;
    assert_eq!(st, 200, "任务列表: {v}");
    v["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["code"] == code)
        .unwrap_or_else(|| panic!("任务列表里应有 {code}（代码注册 + 启动时补种）: {v}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// 直接写库造一行设备（不走登录：每台假设备都要一次 Argon2 太慢）。
/// 时戳一律**在 Rust 侧算**再绑 —— sqlx 会话的时区是 UTC，MySQL 的 NOW() 会差 8 小时。
async fn mk_device(
    pool: &MySqlPool,
    admin_id: i64,
    ua: &str,
    status: i8,
    created_at: chrono::NaiveDateTime,
    last_active_at: chrono::NaiveDateTime,
) -> i64 {
    let id = new_row_id();
    sqlx::query(
        "INSERT INTO admin_login_device (id, admin_id, user_agent, ip, status, created_at, last_active_at) \
         VALUES (?, ?, ?, '127.0.0.1', ?, ?, ?)",
    )
    .bind(id)
    .bind(admin_id)
    .bind(ua)
    .bind(status)
    .bind(created_at)
    .bind(last_active_at)
    .execute(pool)
    .await
    .expect("造设备行失败");
    id
}

async fn exists(pool: &MySqlPool, id: i64) -> bool {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_login_device WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    n == 1
}

#[tokio::test]
async fn device_clean_purges_only_old_kicked_rows() {
    let Some(dsn) = dsn() else { return };
    reset_db(&dsn).await;
    // 调度关掉：任务由测试手动触发，跑几次、什么时候跑都是确定的
    let (_srv, base) = start_server_conf(|c| format!("{c}\n[job]\nenabled = false\n")).await;
    let c = Client::new();
    let admin = login(&base, "admin", "admin123").await.expect("登录失败");
    let pool = MySqlPool::connect(&dsn).await.unwrap();
    let uid = dec_id(&my_id(&base, &admin).await);
    // 登录本身留下的那一行（status=1、刚活跃过）：趁假行还没造，先把它认出来 ——
    // 它也是「在用」，顺带当第 4 个样本（造完假行就分不清哪行是它了）
    let login_dev: i64 = sqlx::query_scalar("SELECT id FROM admin_login_device ORDER BY id DESC LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("登录应留下一条设备行");

    let now = chrono::Local::now().naive_local();
    let days_ago = |n: i64| now - chrono::Duration::days(n);
    let old_kicked = mk_device(&pool, uid, "ua-old-kicked", 0, days_ago(1), days_ago(40)).await;
    let new_kicked = mk_device(&pool, uid, "ua-new-kicked", 0, days_ago(100), days_ago(1)).await;
    let live_old = mk_device(&pool, uid, "ua-live", 1, days_ago(100), days_ago(40)).await;

    let jid = job_id(&c, &base, &admin, "device_clean").await;
    let before = last_log_id(&pool, "device_clean").await;
    trigger(&c, &base, &jid, &admin).await;
    let (status, msg, _ms) = wait_job_log(&pool, "device_clean", before).await;
    // job_log.status：1 = 成功 / 0 = 失败（与 export_task 的 2=完成 不是一套，别混）
    assert_eq!(status, 1, "任务要成功: {msg}");
    assert_eq!(
        msg,
        format!("清理过期登录设备 1 条（已踢且 {KEEP_DAYS} 天没露面）"),
        "四个样本里只该清掉「已踢且久未露面」那一个"
    );
    assert!(!exists(&pool, old_kicked).await, "已踢 + 40 天没露面 → 该清（口径看 last_active_at）");
    assert!(exists(&pool, new_kicked).await, "昨天刚踢 → 窗口内，留着（虽然 created_at 很老）");
    assert!(exists(&pool, live_old).await, "在用行（status=1）一律不碰，哪怕久未露面");
    assert!(exists(&pool, login_dev).await, "当前登录设备的行不碰（token 还挂着它）");

    // 幂等：再跑一次没有可清的了，计数归 0 且仍然成功
    let before2 = last_log_id(&pool, "device_clean").await;
    trigger(&c, &base, &jid, &admin).await;
    let (status2, msg2, _) = wait_job_log(&pool, "device_clean", before2).await;
    assert_eq!(status2, 1, "第二次也要成功: {msg2}");
    assert_eq!(msg2, format!("清理过期登录设备 0 条（已踢且 {KEEP_DAYS} 天没露面）"), "第二轮无事可做");

    // 手动触发这一路也走 job_log 留痕（与调度触发同一个执行体）
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_log WHERE job_code = 'device_clean'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 2, "两趟各留一条执行记录");
}
