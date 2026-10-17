// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! I2（批次 E 修复轮 1）：归属 fencing 的行为面。
//!
//! 现场：一行停在「执行中」、认领与心跳都是 20 分钟前（> STALE_CLAIM_SECS=600），
//! 盘上还留着旧代次的 `.part`——就是「旧执行者活着但超时没心跳」的样子。`export_run`
//! 应把它重新认领（**换掉 started_at**，旧 token 从此作废），新执行者要产出完整文件；
//! 旧代次的半份不能被新执行者碰。
//!
//! 判据分层：`beat`/`finish` 的 SQL 谓词（`AND status = 1 AND started_at = ?`）与
//! 「输家只撤取消/删除行的成品」由 `src/export.rs` 的单测钉住 —— bin crate 的私有 SQL
//! 在集成测试里只能抄一份，抄件对 src 不可证伪。这里验真库上的行为：token 旋转变了、
//! 行是 2、文件内容完整、旧代次 `.part` 活着，随后被 `export_clean` 按名字约定收走。
//!
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use serde_json::Value;

const ID: i64 = 9_100_000_000_000_101;
const EXPORT_DIR: &str = "target/test-uploads/exports";
/// 旧执行者那一代的半份：名字必须是 part_path() 认可的 `{id}.{20 位数字}.csv.part`
const OLD_PART: &str = "9100000000000101.20261017120000000000.csv.part";

fn dt(secs_from_now: i64) -> String {
    (chrono::Local::now().naive_local() + chrono::Duration::seconds(secs_from_now))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn export_dir() -> std::path::PathBuf {
    std::path::Path::new(EXPORT_DIR).to_path_buf()
}

#[tokio::test]
async fn stale_executor_is_fenced_and_new_claim_produces_full_file() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    // 关调度：只让手动触发的这一趟 export_run 去认领，避免后台 tick 抢在前面
    let (_srv, base) = common::start_server_conf(|c| format!("{c}\n[job]\nenabled = false\n")).await;
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");
    let c = reqwest::Client::new();
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── 布置现场：旧执行者（执行中、20 分钟无心跳）+ 旧代次半份 ──
    let old_seen = dt(-1200);
    let admin_id: i64 = sqlx::query_scalar("SELECT id FROM admin WHERE username = 'admin'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO export_task \
         (id, admin_id, username, kind, params, status, file_path, row_count, file_size, progress_rows, \
          msg, created_at, started_at, heartbeat_at, finished_at, expire_at) \
         VALUES (?, ?, 'admin', 'login-log', '{}', 1, '', 0, 0, 8000, '', ?, ?, ?, NULL, ?)",
    )
    .bind(ID)
    .bind(admin_id)
    .bind(dt(-7200))
    .bind(&old_seen)
    .bind(&old_seen) // 心跳也停在 20 分钟前 = 判活失败
    .bind(dt(7 * 86400))
    .execute(&pool)
    .await
    .unwrap();
    let dir = export_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(OLD_PART), b"old-generation-half").unwrap();

    // ── 手动触发 export_run（调度关着，等它把任务派发出去）──
    let resp = c.get(format!("{base}/api/v1/jobs")).bearer_auth(&admin).send().await.unwrap();
    let st = resp.status().as_u16();
    let v = resp.json::<Value>().await.unwrap();
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
    assert_eq!(status, 1, "export_run 派发成功: {msg}");

    // ── 等后台任务跑完（派发是立即返回的）──
    let mut done = None;
    for _ in 0..200 {
        let row: (i8, i64, String, i64) = sqlx::query_as(
            "SELECT status, started_at > ?, file_path, row_count FROM export_task WHERE id = ?",
        )
        .bind(&old_seen)
        .bind(ID)
        .fetch_one(&pool)
        .await
        .unwrap();
        if row.0 == 2 {
            done = Some(row);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let (status, rotated, file_path, rows) = done.expect("导出任务应在 10 秒内跑完");

    // ── 新执行者：换了 token、产出完整文件 ──
    assert_eq!(status, 2, "重新认领后应完成");
    assert_eq!(rotated, 1, "认领必须刷新 started_at —— 旧执行者的 token 从此写不进");
    assert!(rows >= 1, "登录成功至少有一条登录日志可导: row_count={rows}");
    assert_eq!(
        file_path,
        dir.join(format!("{ID}.csv")).to_string_lossy(),
        "终态行里的路径要指向成品"
    );
    let bytes = std::fs::read(&file_path).expect("成品文件应存在");
    assert!(bytes.starts_with(b"\xEF\xBB\xBF"), "CSV 要带 UTF-8 BOM");
    let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("ID,用户名,IP,User-Agent,结果,详情,时间"),
        "表头要与同步导出同一份"
    );
    assert_eq!(text.lines().count() as i64, rows + 1, "行数 = row_count + 表头");
    assert!(text.contains("admin"), "导出内容里应看得到登录账号: {text}");

    // ── 代次隔离：新执行者不碰旧执行者的半份；它自己的半份已改名/清掉 ──
    let old_part = dir.join(OLD_PART);
    assert!(old_part.exists(), "旧代次的 .part 不该被新执行者删掉（代次隔离）");
    let parts: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with(&format!("{ID}.")) && n.ends_with(".csv.part")
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(parts, vec![OLD_PART.to_string()], "新执行者自己的 .part 不该留下: {parts:?}");

    // ── 行已是终态：export_clean 按名字约定把旧代次半份收走 ──
    let resp = c.get(format!("{base}/api/v1/jobs")).bearer_auth(&admin).send().await.unwrap();
    let v = resp.json::<Value>().await.unwrap();
    let clean = v["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["code"] == "export_clean")
        .expect("export_clean 在注册表里");
    let clean_id = common::as_id(&clean["id"]);
    let mark = common::last_log_id(&pool, "export_clean").await;
    common::trigger(&c, &base, &clean_id, &admin).await;
    let (status, msg, _) = common::wait_job_log(&pool, "export_clean", mark).await;
    assert_eq!(status, 1, "清理任务执行成功: {msg}");
    assert!(!old_part.exists(), "终态行的旧代次半份应被收走: {msg}");

    let _ = std::fs::remove_dir_all(&dir);
}
