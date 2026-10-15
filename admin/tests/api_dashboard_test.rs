// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 扩展工程 A4（项 2）真仪表盘：`GET /api/v1/dashboard/stats`。
//! 登录即可（不挂权限码）；计数与近 7 天登录序列要和真库对得上；无数据的日期补 0 不炸。
//! 起真实进程 + 真库（bee_admin_test），需要 `BEE_ADMIN_DB_DSN`。
mod common;

use chrono::{Duration, Local};
use reqwest::Method;
use serde_json::{Value, json};

use common::{dsn, login, new_row_id, reset_db, start_server};

async fn call(
    c: &reqwest::Client,
    method: Method,
    url: String,
    token: Option<&str>,
) -> (u16, Value) {
    let mut rb = c.request(method, url);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    let r = rb.send().await.expect("请求发送失败");
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

/// 直接造登录记录：days_ago 天前（0 = 今天），status 1 成功 / 0 失败。
///
/// 时戳**必须在 Rust 侧算**：sqlx 连接会把会话时区设成 UTC（`SET time_zone='+00:00'`），
/// MySQL 侧的 NOW() 落在 UTC 帧、比应用（chrono::Local）写的本地时戳早 8 小时 ——
/// 凌晨跑测试时整批假数据的日期会整体差一天（踩过）。
async fn fake_login(pool: &sqlx::MySqlPool, days_ago: i64, status: i8) {
    let ts = (Local::now() - Duration::days(days_ago)).naive_local();
    sqlx::query(
        "INSERT INTO login_log (id, admin_id, username, ip, user_agent, status, msg, created_at) \
         VALUES (?, 0, 'ghost', '', '', ?, '', ?)",
    )
    .bind(new_row_id())
    .bind(status)
    .bind(ts)
    .execute(pool)
    .await
    .expect("造登录记录");
}

#[tokio::test]
async fn dashboard_stats() {
    let Some(dsn) = dsn() else { return };
    reset_db(&dsn).await;
    let (_server, base) = start_server().await;
    let c = reqwest::Client::new();

    // 登录即可——没 token 不行
    let (st, _) = call(&c, Method::GET, format!("{base}/api/v1/dashboard/stats"), None).await;
    assert_eq!(st, 401, "仪表盘要登录");

    let admin = login(&base, "admin", "admin123").await.expect("超管登录");
    // 造数据：3 天前 2 成功 1 失败；昨天 1 成功；10 天前 5 成功（窗口外，不该进序列）
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    for _ in 0..2 {
        fake_login(&pool, 3, 1).await;
    }
    fake_login(&pool, 3, 0).await;
    fake_login(&pool, 1, 1).await;
    for _ in 0..5 {
        fake_login(&pool, 10, 1).await;
    }

    let (st, v) = call(&c, Method::GET, format!("{base}/api/v1/dashboard/stats"), Some(&admin)).await;
    assert_eq!(st, 200, "仪表盘: {v}");
    let data = &v["data"];

    // 基础计数：库里只有 seed 的超管
    assert_eq!(data["counts"]["admins"], json!(1), "{v}");
    assert_eq!(data["counts"]["roles"], json!(0), "{v}");
    assert_eq!(data["counts"]["depts"], json!(0), "{v}");
    assert_eq!(data["counts"]["notices"], json!(0), "{v}");

    // 近 7 天序列：7 条、旧→新、最后一条是今天
    let days = data["logins"]["days"].as_array().expect("days 数组");
    assert_eq!(days.len(), 7, "近 7 天: {v}");
    let dates: Vec<&str> = days.iter().map(|d| d["date"].as_str().unwrap_or("")).collect();
    let mut sorted = dates.clone();
    sorted.sort();
    assert_eq!(dates, sorted, "按日期升序: {v}");
    let today = days.last().unwrap();
    assert_eq!(today["success"], json!(1), "今天只有本次登录：{v}");
    assert_eq!(today["fail"], json!(0), "{v}");
    let d3 = &days[3]; // 7 条里 3 天前是倒数第 4 条
    assert_eq!(d3["success"], json!(2), "3 天前 2 次成功: {v}");
    assert_eq!(d3["fail"], json!(1), "3 天前 1 次失败: {v}");
    assert_eq!(days[5]["success"], json!(1), "昨天 1 次成功: {v}");
    // 无数据的日期补 0（days[0] 是最早那天，除非 10 天前那批漏进来）
    assert_eq!(days[0]["success"], json!(0), "窗口外的不计，空日期补 0: {v}");
    assert_eq!(days[0]["fail"], json!(0), "{v}");

    // 总数 = 7 天窗口内的和（10 天前的 5 次不算）
    assert_eq!(data["logins"]["success_total"], json!(4), "今天 1 + 3 天前 2 + 昨天 1: {v}");
    assert_eq!(data["logins"]["fail_total"], json!(1), "{v}");
}
