// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! I1 增补（批次 E 修复轮 1）：并发 2 下**两个任务同跑**的字节对照。
//!
//! 现有跨批对照只覆盖「一个任务跑 6000 行」；这里钉的是两个任务**同时**在跑：
//! 同一 kind（login-log）不同筛选各 6000 行（> `csv::BATCH` 5000，各自都要分两批），
//! `LOCK TABLES login_log WRITE` 把两个执行者一起冻在取数那一步 —— 现场可见两份按
//! **任务 id 分开**的 `.part`（代次段之后各自一份，没有共享文件）。解锁后各自跑完，
//! 两份产物必须分别与**自己筛选条件**的同步导出逐字节一致：错写/交叉都会在这里露馅。
//!
//! 单开一个文件：集成测试是「一文件一 `#[tokio::test]`」（cargo 串行跑二进制），
//! 而 api_export_cancel_test.rs 已 ~460 行、家规 500 行上限。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;

const EXP: &str = "/exports";
/// 导出目录（`upload_dir` 是 cwd 相对的，conf/app.conf.test 里就是 target/test-uploads）。
const OUT: &str = "target/test-uploads/exports";
const N: i64 = 6000;

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

async fn download(c: &reqwest::Client, url: String, token: &str) -> (u16, Vec<u8>) {
    let r = c.get(url).bearer_auth(token).send().await.expect("下载请求失败");
    (r.status().as_u16(), r.bytes().await.unwrap().to_vec())
}

async fn sync_csv(c: &reqwest::Client, url: String, token: &str) -> Vec<u8> {
    let r = c.get(url).bearer_auth(token).send().await.expect("同步导出请求失败");
    assert_eq!(r.status(), 200, "同步导出必须仍可用（回归）");
    r.bytes().await.unwrap().to_vec()
}

/// 直接写库造 n 行登录记录（不走登录接口：n 次 Argon2 校验慢到不可接受）。
/// 时戳由调用方在 Rust 侧算（sqlx 会话时区是 UTC，MySQL NOW() 会偏 8 小时）。
async fn bulk_login_logs(pool: &sqlx::MySqlPool, username: &str, n: i64, at: chrono::NaiveDateTime, mut id: i64) {
    let at = at.format("%Y-%m-%d %H:%M:%S").to_string();
    for _ in 0..(n / 1000) {
        let mut sql = String::from(
            "INSERT INTO login_log (id, admin_id, username, ip, user_agent, status, msg, created_at) VALUES ",
        );
        for i in 0..1000 {
            if i > 0 {
                sql.push(',');
            }
            sql.push_str(&format!("({id}, 0, '{username}', '127.0.0.1', 'bulk', 0, '批量造数', '{at}')"));
            id += 1;
        }
        sqlx::query(&sql).execute(pool).await.expect("批量造登录记录");
    }
}

fn parts_of(id: i64) -> Vec<std::path::PathBuf> {
    let prefix = format!("{id}.");
    let Ok(rd) = std::fs::read_dir(OUT) else { return Vec::new() };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with(&prefix) && name.ends_with(".csv.part")
        })
        .collect()
}

/// 等任务的半份文件出现（上限 10 秒）。执行者被表锁冻在取数那一步，
/// 文件只可能**出现**（冻住之后不会再消失），等一下是确定的，不是赌时序。
async fn wait_part(id: i64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while parts_of(id).is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "10 秒内没等到任务 {id} 的 .part");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn snapshot(pool: &sqlx::MySqlPool, id: i64) -> (i8, i64, i64, String) {
    sqlx::query_as("SELECT status, row_count, progress_rows, msg FROM export_task WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("任务行还在")
}

async fn wait_status(pool: &sqlx::MySqlPool, id: i64, want: &[i8]) -> (i8, i64, i64, String) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let now = snapshot(pool, id).await;
        if want.contains(&now.0) {
            return now;
        }
        assert!(tokio::time::Instant::now() < deadline, "等状态 {want:?} 超时，现在 {now:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 清掉这两个 id 在导出目录里的所有旧产物：reset_db 会把 AUTO_INCREMENT 拨回 1，
/// 上一个测试二进制可能留下了同 id 的文件；不先清，现场就不是本用例的了。
fn clean_artifacts(ids: &[i64]) {
    for id in ids {
        let prefix = format!("{id}.");
        let Ok(rd) = std::fs::read_dir(OUT) else { continue };
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

async fn job_id(c: &reqwest::Client, base: &str, token: &str, code: &str) -> String {
    let (st, v) = call(c, Method::GET, format!("{base}/api/v1/jobs?size=100"), Some(token), None).await;
    assert_eq!(st, 200, "任务列表: {v}");
    v["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["code"] == code)
        .unwrap_or_else(|| panic!("任务列表里应有 {code}: {v}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn parallel_exports_match_their_own_sync_exports_byte_for_byte() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    // 关掉调度循环：本用例精确控制「什么时候跑」，也保证锁表期间没有别的 tick 插进来
    let (_srv, base) = common::start_server_conf(|c| format!("{c}\n[job]\nenabled = false\n")).await;
    let c = reqwest::Client::new();
    let url = |p: &str| format!("{base}/api/v1{p}");
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");
    let run_id = job_id(&c, &base, &admin, "export_run").await;
    std::fs::create_dir_all(OUT).expect("导出目录");

    // 两套互不为子串的数据：筛选各自取 6000 行，都跨 BATCH(5000) —— 必然各跑两批
    let fresh = chrono::Local::now().naive_local();
    // new_row_id 每次只推进 1，而一批要占 N 个主键；两批从同一起点相邻取号必撞主键
    let base_id = common::new_row_id();
    bulk_login_logs(&pool, "e2_par_a", N, fresh - chrono::Duration::days(1), base_id).await;
    bulk_login_logs(&pool, "e2_par_b", N, fresh - chrono::Duration::days(2), base_id + N).await;

    // 发起两个任务（同 kind、不同筛选），记下对外 id
    let mut tasks: Vec<(i64, String, String)> = Vec::new();
    for who in ["e2_par_a", "e2_par_b"] {
        let (st, t) = call(&c, Method::POST, url(EXP), Some(&admin),
            Some(json!({"kind": "login-log", "params": {"username": who}}))).await;
        assert_eq!(st, 200, "{t}");
        let id = common::dec_id(&common::as_id(&t["data"]["id"]));
        tasks.push((id, common::as_id(&t["data"]["id"]), who.to_string()));
    }
    let ids: Vec<i64> = tasks.iter().map(|t| t.0).collect();
    clean_artifacts(&ids);

    // ── 冻住取数：两个执行者同时被认领，盘上各一份 .part ──
    let mut lock = pool.acquire().await.unwrap();
    sqlx::raw_sql("LOCK TABLES login_log WRITE").execute(&mut *lock).await.expect("锁表");
    let before = common::last_log_id(&pool, "export_run").await;
    common::trigger(&c, &base, &run_id, &admin).await;
    let (jstatus, jmsg, _) = common::wait_job_log(&pool, "export_run", before).await;
    assert_eq!(jstatus, 1, "{jmsg}");
    assert_eq!(jmsg, "导出任务 2 个（已派发，并发上限 2）", "两个任务都该被派发: {jmsg}");
    for id in &ids {
        let (status, _, _, _) = wait_status(&pool, *id, &[1]).await;
        assert_eq!(status, 1, "任务 {id} 应已被认领");
        wait_part(*id).await;
    }
    let part_a = parts_of(ids[0]);
    let part_b = parts_of(ids[1]);
    assert_eq!((part_a.len(), part_b.len()), (1, 1),
        "两个任务各一份半份（按 id 隔离）: {part_a:?} / {part_b:?}");
    assert_ne!(part_a[0], part_b[0], "半份文件不能共用: {:?}", part_a[0]);

    // ── 解锁，两个任务并排跑完 ──
    sqlx::raw_sql("UNLOCK TABLES").execute(&mut *lock).await.unwrap();
    drop(lock);
    let mut out = Vec::new();
    for (id, sid, who) in &tasks {
        let (status, rows, prog, msg) = wait_status(&pool, *id, &[2, 3]).await;
        assert_eq!((status, rows, prog), (2, N, N), "任务 {id}（{who}）两批跑完: {msg}");
        assert!(parts_of(*id).is_empty(), "跑完不留半份: {:?}", parts_of(*id));
        let (st, bytes) = download(&c, url(&format!("{EXP}/{sid}/download")), &admin).await;
        assert_eq!(st, 200, "任务 {id} 的成品能下");
        out.push(bytes);
    }

    // 负控：两组数据不同，产物也应不同 —— 相同说明两个任务读到同一份数据（交叉）
    assert_ne!(out[0], out[1], "两个筛选的产物不该相同（数据隔离）");
    // 正控：各自 == 自己筛选条件的同步导出，逐字节（错写/串数据都过不了）
    for (i, (_, _, who)) in tasks.iter().enumerate() {
        let other = if who.ends_with('a') { "e2_par_b" } else { "e2_par_a" };
        let text = String::from_utf8_lossy(&out[i]);
        assert!(text.contains(who) && !text.contains(other),
            "产物 {i} 只该含自己的 {who}，不该含 {other}");
        assert_eq!(out[i], sync_csv(&c, url(&format!("/login-logs/export?username={who}")), &admin).await,
            "并发任务（{who}）== 对应同步导出");
    }
    clean_artifacts(&ids);
}
