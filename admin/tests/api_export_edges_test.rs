// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 导出的两条闭口边界（批次 E T3）与两条索引（T4）：
//!
//! - **T3a 发起人被删**：执行时按发起人**当前**身份重算数据范围，人没了就整单失败
//!   （status=3 + 「发起人已不存在」），不许拿旧快照把数据导出去。行是直接插库的 ——
//!   这正是接口（正常路径）造不出来的那种行。
//! - **T3b 库里残留未知 kind**：`kind` 是代码注册键，KINDS 里没有 = 任务失败（与
//!   `job.code` 同一口径），不许静默跳过、更不许回落到某个默认类型。
//! - **T4 索引**：`export_run` 每分钟按 status 找待处理/卡住的行、`export_clean` 每天按
//!   expire_at 找过期的行 —— 两条各配一条索引；用 `SHOW INDEX` 实测（information_schema
//!   口径之外再看一眼 MySQL 自己怎么说）。
//!
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{json, Value};
use sqlx::Row;

/// 手写雪花样 id：和真发号器（时间戳高位）几乎不可能撞，测试里一眼认得出。
const ID_OWNER_GONE: i64 = 9_000_000_000_000_001;
const ID_GHOST_KIND: i64 = 9_000_000_000_000_002;

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
    (status, r.json().await.unwrap_or(Value::Null))
}

fn dt(secs_from_now: i64) -> String {
    (chrono::Local::now().naive_local() + chrono::Duration::seconds(secs_from_now))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// 直接插一条 export_task（跳过接口）：要造的行恰恰是接口造不出来的那种。
async fn insert_task(pool: &sqlx::MySqlPool, id: i64, admin_id: i64, username: &str, kind: &str) {
    sqlx::query(
        "INSERT INTO export_task \
         (id, admin_id, username, kind, params, status, file_path, row_count, file_size, progress_rows, msg, \
          created_at, started_at, finished_at, expire_at) \
         VALUES (?, ?, ?, ?, '{}', 0, '', 0, 0, 0, '', ?, NULL, NULL, ?)",
    )
    .bind(id)
    .bind(admin_id)
    .bind(username)
    .bind(kind)
    .bind(dt(0))
    .bind(dt(3600))
    .execute(pool)
    .await
    .unwrap();
}

/// 行现状：(status, msg)。
async fn task_state(pool: &sqlx::MySqlPool, id: i64) -> (i8, String) {
    sqlx::query_as::<_, (i8, String)>("SELECT status, msg FROM export_task WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("export_task 行还在（任务失败也要留行给发起人看原因）")
}

#[tokio::test]
async fn export_run_fails_closed_and_indexes_exist() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    // 关调度：不然 10 秒内的 tick 会替我们把任务跑掉，触发与断言的时序就说不清了
    let (_srv, base) = common::start_server_conf(|c| format!("{c}\n[job]\nenabled = false\n")).await;
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");
    let c = reqwest::Client::new();
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── T3a：给一个「马上要被删掉」的发起人建任务 ──
    let (st, v) = call(
        &c,
        Method::POST,
        format!("{base}/api/v1/admins"),
        Some(&admin),
        Some(json!({"username": "gone_u", "password": "gone_u12345", "nickname": "待删用户"})),
    )
    .await;
    assert_eq!(st, 200, "建发起人: {v}");
    let gone_short = common::as_id(&v["data"]["id"]);
    let gone_id = common::dec_id(&gone_short);
    insert_task(&pool, ID_OWNER_GONE, gone_id, "gone_u", "audit-log").await;
    let (st, v) = call(
        &c,
        Method::DELETE,
        format!("{base}/api/v1/admins/{gone_short}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(st, 200, "删掉发起人: {v}");

    // ── T3b：库里残留一个已经下线的 kind（发起人正常，先卡在 kind 上）──
    insert_task(&pool, ID_GHOST_KIND, common::dec_id(&common::my_id(&base, &admin).await), "admin", "ghost-kind").await;

    // ── 跑一趟 export_run：两单都必须失败，且原因各说各的 ──
    let (st, v) = call(&c, Method::GET, format!("{base}/api/v1/jobs"), Some(&admin), None).await;
    assert_eq!(st, 200, "任务列表: {v}");
    let job = v["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["code"] == "export_run")
        .expect("export_run 在注册表里");
    let job_id = common::as_id(&job["id"]);
    let mark = common::last_log_id(&pool, "export_run").await;
    common::trigger(&c, &base, &job_id, &admin).await;
    let (status, msg, _) = common::wait_job_log(&pool, "export_run", mark).await;
    assert_eq!(status, 1, "任务本身跑完（单笔失败不是任务失败）: {msg}");
    // 派发式（E2-1）：任务消息只说派了几单；每单的成败落在**各自的行**上，
    // 所以下面要等行到终态再断言（任务消息回来时它们可能还在跑）
    assert_eq!(msg, "导出任务 2 个（已派发，并发上限 2）", "派发完就返回: {msg}");
    for id in [ID_OWNER_GONE, ID_GHOST_KIND] {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let (s, _) = task_state(&pool, id).await;
            if s == 3 {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "任务 {id} 60 秒内没到失败态（现在 {s}）");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    let (status, msg) = task_state(&pool, ID_OWNER_GONE).await;
    assert_eq!(status, 3, "发起人没了 → 3 失败（绝不拿旧快照导出）: {msg}");
    assert!(msg.contains("发起人已不存在"), "失败原因要说清是人没了: {msg}");

    let (status, msg) = task_state(&pool, ID_GHOST_KIND).await;
    assert_eq!(status, 3, "未知 kind → 3 失败: {msg}");
    assert!(msg.contains("未知导出类型"), "失败原因要点名 kind: {msg}");
    // 失败的任务不留半份文件：`.part` 由 run_one 的看守清掉，`.csv` 根本不该产生
    let dir = std::path::Path::new("target/test-uploads/exports");
    for id in [ID_OWNER_GONE, ID_GHOST_KIND] {
        assert!(!dir.join(format!("{id}.csv")).exists(), "失败不留成品 {id}");
        assert!(!dir.join(format!("{id}.csv.part")).exists(), "失败不留半份 {id}");
    }

    // ── T4：两条索引（SHOW INDEX 实测）──
    let rows = sqlx::query("SHOW INDEX FROM export_task")
        .fetch_all(&pool)
        .await
        .expect("SHOW INDEX 能跑");
    let pick = |name: &str| -> Vec<(String, u64)> {
        let mut cols: Vec<(String, u64)> = rows
            .iter()
            .filter(|r| r.try_get::<String, _>("Key_name").map(|k| k == name).unwrap_or(false))
            .map(|r| {
                (
                    r.try_get::<String, _>("Column_name").expect("Column_name"),
                    // Seq_in_index 在 MySQL 里是 INT UNSIGNED，解成 i64 会 ColumnDecode
                    r.try_get::<u64, _>("Seq_in_index").expect("Seq_in_index"),
                )
            })
            .collect();
        cols.sort_by_key(|(_, seq)| *seq);
        cols
    };
    assert_eq!(
        pick("idx_export_task_status"),
        vec![("status".to_string(), 1)],
        "export_run 每分钟按 status 扫，必须有条 status 索引（且是单列）"
    );
    assert_eq!(
        pick("idx_export_task_expire_at"),
        vec![("expire_at".to_string(), 1)],
        "export_clean 每天按 expire_at 扫，必须有条 expire_at 索引（且是单列）"
    );
}
