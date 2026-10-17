// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

use super::retention::{too_fresh, ATTACH_MIN_AGE_SECS};
use super::*;
use chrono::NaiveDateTime;

fn job(cron: &str, last: Option<&str>) -> Job {
    Job {
        id: 1,
        name: "t".into(),
        code: "log_retention".into(),
        cron: cron.into(),
        status: 1,
        last_run_at: last.map(|s| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
        }),
        last_status: None,
        last_msg: String::new(),
        created_at: NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        updated_at: NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
    }
}

fn at(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
}

#[test]
fn due_compares_interval_against_last_run() {
    let now = at("2026-10-06 12:00:00");
    assert!(due(&job("3600", None), now), "没跑过 = 到期");
    assert!(!due(&job("3600", Some("2026-10-06 11:30:00")), now), "还没到间隔");
    assert!(due(&job("3600", Some("2026-10-06 10:59:00")), now), "过了间隔");
    assert!(due(&job("3600", Some("2026-10-06 11:00:00")), now), "正好到点也要跑");
    // 手工把间隔改坏 = 永不到期（而不是每个 tick 都跑）
    assert!(!due(&job("abc", None), now));
    assert!(!due(&job("0", None), now));
    assert!(!due(&job("-5", None), now));
}

#[test]
fn interval_parsing_rejects_garbage() {
    assert_eq!(interval_secs("3600"), Some(3600));
    assert_eq!(interval_secs(" 2 "), Some(2));
    assert_eq!(interval_secs("0"), None);
    assert_eq!(interval_secs("-1"), None);
    assert_eq!(interval_secs("3.5"), None);
    assert_eq!(interval_secs(""), None);
    assert_eq!(interval_secs("1h"), None);
}

/// 「已在跑」闸：同一 code 只放一个进去（连点触发不能叠出第二份任务）。
#[test]
fn busy_gate_holds_only_one_run_per_code() {
    let held = Busy::acquire("log_retention").expect("第一次能占上");
    assert!(Busy::acquire("log_retention").is_none(), "同一 code 第二次占不上");
    let other = Busy::acquire("export_run").expect("别的 code 不受影响");
    drop(held);
    assert!(Busy::acquire("log_retention").is_some(), "释放后能再占");
    assert!(Busy::acquire("export_run").is_none(), "释放一个不影响另一个");
    drop(other);
    assert!(Busy::acquire("export_run").is_some(), "另一个也释放后能再占");
}

/// S2 保护期：新文件（含读不到 mtime 的）不碰，过了保护期的才收 —— 这条测试钉的是
/// 「上传落盘 → INSERT」窗口与「删行后延迟回收」两处都靠它兜底。
#[test]
fn orphan_guard_only_collects_settled_files() {
    let dir = std::env::temp_dir().join(format!("bee-guard-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let fresh = dir.join("fresh.png");
    std::fs::write(&fresh, b"x").unwrap();
    assert!(too_fresh(&fresh), "刚写下的文件在保护期内");

    let old = dir.join("old.png");
    std::fs::write(&old, b"x").unwrap();
    let aged = std::time::SystemTime::now()
        - std::time::Duration::from_secs(ATTACH_MIN_AGE_SECS + 60);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(aged)
        .unwrap();
    assert!(!too_fresh(&old), "过了保护期就是可收的孤儿");

    assert!(too_fresh(&dir.join("gone.png")), "读不到 mtime 一律当新（不删）");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn registry_codes_are_unique_and_looked_up() {
    assert!(spec("log_retention").is_some());
    assert!(spec("avatar_orphan_clean").is_some());
    assert!(spec("db_backup").is_some());
    assert!(spec("export_run").is_some());
    assert!(spec("export_clean").is_some());
    assert!(spec("device_clean").is_some());
    assert!(spec("ghost_job").is_none(), "库里残留的旧 code 找不到注册项");
    let mut codes: Vec<&str> = JOBS.iter().map(|j| j.code).collect();
    codes.sort_unstable();
    let n = codes.len();
    codes.dedup();
    assert_eq!(codes.len(), n, "注册表里的 code 不能重复");
}
