// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 批次 E 阶段 3 · A5：手动触发闸改成 **acquire-first**。
//! 旧实现是「接口先 is_running 预检 → spawn 里再 acquire」，两个请求可能都过预检：
//! 后一个被接口回了 200，但它的任务体在 `jobs::run` 里拿不到闸、静默跳过 ——
//! 用户看到「已触发」却什么都没执行。现在接口在 spawn **之前**拿 permit，
//! 拿不到当场 409，permit 随任务走到结束。
//!
//! 用 `db_backup` 当慢任务（真 mysqldump，本机实测 ~134ms），同时发 16 个触发：
//! 必须恰好 1 个 200、15 个 409，且这一轮只多一条 job_log。
//! 起真实进程 + 真库（bee_admin_test），需要 `BEE_ADMIN_DB_DSN`。
mod common;

use serde_json::Value;

/// 某任务的执行记录条数（重置库后从 0 起算）。
async fn job_log_count(pool: &sqlx::MySqlPool, code: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM job_log WHERE job_code = ?")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn concurrent_triggers_accept_exactly_one() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    // 关调度：本轮只测手动触发这一路，避免 tick 的自动执行撞上闸
    let (_server, base) = common::start_server_conf(|c| format!("{c}\n[job]\nenabled = false\n")).await;
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let c = reqwest::Client::new();
    let token = common::login(&base, "admin", "admin123").await.expect("超管登录");

    let v: Value = c
        .get(format!("{base}/api/v1/jobs?size=100"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("任务列表请求失败")
        .json()
        .await
        .expect("任务列表不是 JSON");
    let id = v["data"]["list"]
        .as_array()
        .and_then(|l| l.iter().find(|j| j["code"] == "db_backup"))
        .map(|j| j["id"].as_str().unwrap_or_default().to_string())
        .unwrap_or_else(|| panic!("任务列表里应有 db_backup: {v}"));
    let url = format!("{base}/api/v1/jobs/{id}/run");

    for round in 1..=3 {
        let before_logs = job_log_count(&pool, "db_backup").await;
        let mark = common::last_log_id(&pool, "db_backup").await;

        // 16 个请求同时发：acquire-first 下第一个拿到 permit，其余当场 409
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let c = c.clone();
            let url = url.clone();
            let token = token.clone();
            tasks.push(tokio::spawn(async move {
                c.post(&url)
                    .bearer_auth(&token)
                    .send()
                    .await
                    .expect("触发请求失败")
                    .status()
                    .as_u16()
            }));
        }
        let mut statuses = Vec::new();
        for t in tasks {
            statuses.push(t.await.unwrap());
        }
        let ok = statuses.iter().filter(|s| **s == 200).count();
        let busy = statuses.iter().filter(|s| **s == 409).count();
        assert_eq!(ok, 1, "第 {round} 轮：并发触发只能有一者受理，实际 {statuses:?}");
        assert_eq!(busy, 15, "第 {round} 轮：其余 15 个必须 409，实际 {statuses:?}");

        // 被拒的请求不执行、不留 job_log；等受理的那一次跑完，恰好 +1 条
        let (status, msg, _ms) = common::wait_job_log(&pool, "db_backup", mark).await;
        assert_eq!(status, 1, "第 {round} 轮：受理的任务要跑成功: {msg}");
        let after_logs = job_log_count(&pool, "db_backup").await;
        assert_eq!(
            after_logs,
            before_logs + 1,
            "第 {round} 轮：只允许受理者留下一条执行记录（被拒的不许执行）"
        );
    }
}
