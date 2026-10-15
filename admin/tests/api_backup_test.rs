// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 扩展工程 B1（项 9）数据库备份：手动触发 db_backup → 真跑一次 mysqldump →
//! dump 非空且头部是 MySQL dump（可被 mysql 读）→ 保留策略删旧（keep=7）。
//! 起真实进程 + 真库（bee_admin_test），需要 `BEE_ADMIN_DB_DSN`。
mod common;

use std::path::PathBuf;

use serde_json::Value;

use common::{dsn, login, reset_db, start_server_conf};

/// 备份目录 = `[backup] dir` 缺省 → `{upload_dir}/backups`；
/// app.conf.test 的 upload_dir = target/test-uploads。
const BACKUP_DIR: &str = "target/test-uploads/backups";
/// `[backup] keep` 的代码默认值（app.conf.test 没写 [backup]）。
const KEEP: usize = 7;

fn backup_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("备份目录应在任务跑完后存在")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with("backup_") && n.ends_with(".sql")
            })
        })
        .collect();
    out.sort();
    out
}

/// 等后台任务跑完：轮询任务行的「上次结果 / 消息」（后台执行不等请求，只能回头看）。
async fn wait_job_done(c: &reqwest::Client, base: &str, token: &str, code: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        let v: Value = c
            .get(format!("{base}/api/v1/jobs?size=100"))
            .bearer_auth(token)
            .send()
            .await
            .expect("任务列表请求失败")
            .json()
            .await
            .expect("任务列表不是 JSON");
        if let Some(j) = v["data"]["list"]
            .as_array()
            .and_then(|l| l.iter().find(|j| j["code"] == code))
        {
            if let Some(msg) = j["last_msg"].as_str().filter(|m| !m.is_empty()) {
                return msg.to_string();
            }
        }
        assert!(std::time::Instant::now() < deadline, "等 {code} 跑完超时");
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

#[tokio::test]
async fn db_backup_dumps_and_prunes() {
    let Some(dsn) = dsn() else { return };
    reset_db(&dsn).await;
    // 关调度：备份是分钟级任务，tick 的自动备份会和手动触发撞「已在执行中」的闸
    let (_server, base) = start_server_conf(|c| format!("{c}\n[job]\nenabled = false\n")).await;
    let c = reqwest::Client::new();
    let token = login(&base, "admin", "admin123").await.expect("超管登录");

    // 预置 8 份「旧备份」（keep=7）——最旧的至少要被删；再放一个不该被碰的干扰文件
    let dir = std::path::Path::new(BACKUP_DIR);
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let old: Vec<String> = (1..=8).map(|i| format!("backup_2020010{i}_000000.sql")).collect();
    for n in &old {
        std::fs::write(dir.join(n), "老备份占位").unwrap();
    }
    std::fs::write(dir.join("notes.txt"), "别动我").unwrap();

    // 找到代码注册的 db_backup 任务（列表里按 code 认，不猜 id）
    let v: Value = c
        .get(format!("{base}/api/v1/jobs"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("任务列表请求失败")
        .json()
        .await
        .expect("任务列表不是 JSON");
    let job = v["data"]["list"]
        .as_array()
        .expect("list 数组")
        .iter()
        .find(|j| j["code"] == "db_backup")
        .unwrap_or_else(|| panic!("任务列表里应有 db_backup（种子未注册？）: {v}"));
    let job_id = job["id"].as_str().expect("任务 id 是短串").to_string();

    // 手动触发（后台执行：立即返回「已触发」，跑完的结果在 job_log / 任务行里）
    common::trigger(&c, &base, &job_id, &token).await;
    let msg = wait_job_done(&c, &base, &token, "db_backup").await;
    assert!(msg.contains("备份"), "任务行应写明白备份结果: {msg}");

    // ── 保留策略：backup_*.sql 恰好留 KEEP 份，最旧的被删，名字不匹配的文件不动 ──
    let files = backup_files(dir);
    assert_eq!(files.len(), KEEP, "应保留 {KEEP} 份: {files:?}");
    assert!(
        !dir.join(&old[0]).exists(),
        "最旧的备份必须被删: {}",
        old[0]
    );
    assert!(dir.join("notes.txt").exists(), "名字不匹配的文件不该被删");
    // 最新一份必须是本次新生成的（时间戳是今天，排在最旧占位之后）
    let newest = files.last().expect("至少一份备份").clone();
    let name = newest.file_name().unwrap().to_string_lossy().to_string();
    assert!(name > old[7], "最新一份应是本次生成的: {name}");

    // ── dump 内容：非空 + 头部是 MySQL dump（mysql 客户端认的格式）──
    let content = std::fs::read(&newest).expect("读 dump 失败");
    assert!(!content.is_empty(), "dump 不能是空文件");
    let head = String::from_utf8_lossy(&content[..content.len().min(300)]).to_string();
    assert!(head.contains("MySQL dump"), "头部应是 mysqldump 输出: {head}");
    let body = String::from_utf8_lossy(&content);
    assert!(body.contains("CREATE TABLE `admin`"), "应含真实表结构（管理员表）");
}
