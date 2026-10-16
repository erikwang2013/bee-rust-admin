// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! T5（批次 E）：`{id}.csv.part` 的收尾 —— 半份导出文件的最后一个窄口。
//!
//! `.part` 正常有两条归宿：跑完改名成 `.csv`、跑挂由 `PartFile` 看守清掉。兜不住的是
//! 「进程被 SIGKILL（Drop 根本没机会跑）」和「运行中的行被管理员删掉」——半份文件从此
//! 没人认领。`export_clean` 除了删过期行，还要认这几个名字约定：**行没了 / 行已终态 /
//! 行卡在执行中且过期又超时**才收，正在跑的和待处理的绝不碰（碰了就是把手上的活拆了）。
//!
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use serde_json::Value;

const EXPORT_DIR: &str = "target/test-uploads/exports";

const ID_GONE: i64 = 9_100_000_000_000_001;
const ID_DEAD: i64 = 9_100_000_000_000_002;
const ID_STALE: i64 = 9_100_000_000_000_003;
const ID_LIVE: i64 = 9_100_000_000_000_004;
const ID_PENDING: i64 = 9_100_000_000_000_005;
const ID_DONE: i64 = 9_100_000_000_000_006;

fn dt(secs_from_now: i64) -> String {
    (chrono::Local::now().naive_local() + chrono::Duration::seconds(secs_from_now))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// 插一条 export_task（`started_at` 只有执行中的行才有）。发起人用超管自己（真的存在）。
async fn insert_task(
    pool: &sqlx::MySqlPool,
    id: i64,
    status: i8,
    started_at: Option<String>,
    file_path: &str,
) {
    sqlx::query(
        "INSERT INTO export_task \
         (id, admin_id, username, kind, params, status, file_path, row_count, file_size, progress_rows, msg, \
          created_at, started_at, finished_at, expire_at) \
         VALUES (?, 1, 'admin', 'audit-log', '{}', ?, ?, 0, 0, 0, '', ?, ?, NULL, ?)",
    )
    .bind(id)
    .bind(status)
    .bind(file_path)
    .bind(dt(-7200))
    .bind(started_at)
    .bind(dt(-3600)) // 一律已过期：clean 的过期口径不该成为 .part 收集的门槛
    .execute(pool)
    .await
    .unwrap();
}

fn part(id: i64) -> std::path::PathBuf {
    std::path::Path::new(EXPORT_DIR).join(format!("{id}.csv.part"))
}

#[tokio::test]
async fn export_clean_collects_orphaned_part_files_only() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    // 关调度：export_run 的 tick 会把「卡住的」行重跑掉，.part 的布置就白做了
    let (_srv, base) = common::start_server_conf(|c| format!("{c}\n[job]\nenabled = false\n")).await;
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");
    let c = reqwest::Client::new();
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── 布置现场 ──
    let dir = std::path::Path::new(EXPORT_DIR);
    std::fs::create_dir_all(dir).unwrap();
    // 1) 行没了（运行中被删）：半份文件从此没人认领 → 该收
    std::fs::write(part(ID_GONE), b"half").unwrap();
    // 2) 行是终态（SIGKILL 在回写 3 与看守之间）：残留 → 该收
    insert_task(&pool, ID_DEAD, 3, Some(dt(-1800)), "").await;
    std::fs::write(part(ID_DEAD), b"half").unwrap();
    // 3) 行卡在「执行中」、过期又超时（认领超时 10 分钟）→ 那是崩过的活，半份文件该收
    insert_task(&pool, ID_STALE, 1, Some(dt(-1200)), "").await;
    std::fs::write(part(ID_STALE), b"half").unwrap();
    // 4) 正在跑（刚认领）：半份文件就是它手上的活 → 绝不能碰
    insert_task(&pool, ID_LIVE, 1, Some(dt(0)), "").await;
    std::fs::write(part(ID_LIVE), b"half").unwrap();
    // 5) 待处理（还没跑，马上轮到它）：.part 不该存在，但真存在也不能先拆了 →
    //    它会自己 truncate 重写这份文件
    insert_task(&pool, ID_PENDING, 0, None, "").await;
    std::fs::write(part(ID_PENDING), b"half").unwrap();
    // 6) 过期已完成的：行与 .csv 都要按老口径删掉，残留 .part 一并收（同一轮里）
    let done_csv = dir.join(format!("{ID_DONE}.csv"));
    std::fs::write(&done_csv, b"a,b\n").unwrap();
    std::fs::write(part(ID_DONE), b"half").unwrap();
    insert_task(&pool, ID_DONE, 2, None, done_csv.to_str().unwrap()).await;
    // 7) 名字不是约定的（非数字 id / 不是 .csv.part）：名字对不上就永远不该动
    std::fs::write(dir.join("abc.csv.part"), b"x").unwrap();
    std::fs::write(dir.join("readme.txt"), b"x").unwrap();

    // ── 跑一趟 export_clean ──
    let resp = c
        .get(format!("{base}/api/v1/jobs"))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    let st = resp.status().as_u16();
    let v = resp.json::<Value>().await.unwrap();
    assert_eq!(st, 200, "任务列表: {v}");
    let job = v["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["code"] == "export_clean")
        .expect("export_clean 在注册表里");
    let job_id = common::as_id(&job["id"]);
    let mark = common::last_log_id(&pool, "export_clean").await;
    common::trigger(&c, &base, &job_id, &admin).await;
    let (status, msg, _) = common::wait_job_log(&pool, "export_clean", mark).await;
    assert_eq!(status, 1, "清理任务执行成功: {msg}");
    assert!(msg.contains("收走残留 .part 4 个"), "四个该收的都收了: {msg}");

    // ── 该收的：没了 ──
    for (id, why) in [
        (ID_GONE, "行已被删"),
        (ID_DEAD, "行已终态"),
        (ID_STALE, "行卡在执行中且过期超时"),
        (ID_DONE, "终态行 + 已过期"),
    ] {
        assert!(!part(id).exists(), "{why} → .part 收走 [{id}]: {msg}");
    }
    assert!(!done_csv.exists(), "过期已完成行的 .csv 照旧删（老口径不变）: {msg}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_task WHERE id = ?")
        .bind(ID_DONE)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "过期已完成的行照旧删: {msg}");

    // ── 不该碰的：还在 ──
    assert!(part(ID_LIVE).exists(), "正在跑的行，半份文件是它手上的活: {msg}");
    assert!(part(ID_PENDING).exists(), "待处理的行，别抢在它开跑前拆文件: {msg}");
    assert!(dir.join("abc.csv.part").exists(), "名字不是 {{id}}.csv.part 的一律不碰: {msg}");
    assert!(dir.join("readme.txt").exists(), "非导出文件一律不碰: {msg}");
    let stale_row: (i8, String) = sqlx::query_as("SELECT status, msg FROM export_task WHERE id = ?")
        .bind(ID_STALE)
        .fetch_one(&pool)
        .await
        .expect("卡住的行清理时不动它（等 export_run 重跑）");
    assert_eq!(stale_row.0, 1, "行的状态不被清理任务改: {stale_row:?}");

    // ── 再跑一趟：全都收完了，收尾幂等（第二次 0 个）──
    let mark = common::last_log_id(&pool, "export_clean").await;
    common::trigger(&c, &base, &job_id, &admin).await;
    let (status, msg, _) = common::wait_job_log(&pool, "export_clean", mark).await;
    assert_eq!(status, 1, "第二轮清理: {msg}");
    assert!(msg.contains("收走残留 .part 0 个"), "第二轮没有可收的了: {msg}");
    let _ = std::fs::remove_dir_all(dir);
}
