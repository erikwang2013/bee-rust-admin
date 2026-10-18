// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

use super::clean::{collectable, final_files, part_files};
use super::kinds::KINDS;
use super::run::{
    beat_sql, claim_sql, dispatch, final_path, finish_sql, gate, part_path, should_remove_artifact,
};
use super::*;
use chrono::NaiveDateTime;

#[test]
fn kinds_are_unique_and_looked_up_by_code() {
    assert_eq!(spec("audit-log").map(|k| k.perm), Some("system:auditlog:list"));
    assert_eq!(spec("login-log").map(|k| k.file_prefix), Some("login-logs"));
    assert!(spec("nope").is_none(), "没注册的类型必须找不到");
    let mut codes: Vec<&str> = KINDS.iter().map(|k| k.code).collect();
    codes.sort_unstable();
    let n = codes.len();
    codes.dedup();
    assert_eq!(codes.len(), n, "注册表里的 code 不能重复");
}

/// 名字约定：旧格式 `{id}.csv.part` 与 E2 起的 `{id}.{20 位代次}.csv.part` 才是回收
/// 对象；别的名字（运维手放的、别的进程写的、代次不是 20 位数字的）一律不看；
/// 目录不存在 = 空手而归而不是报错。
#[test]
fn part_files_only_matches_the_naming_convention() {
    let dir = std::env::temp_dir().join(format!("bee-parts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in [
        "7.csv.part",                       // 旧格式（升级前残留 / 手工兜底）
        "8.20261017123456123456.csv.part",  // 新格式：{id}.{代次}.csv.part
        "abc.csv.part",                     // id 不是数字
        "readme.txt",
        "9.csv",
        "10.123.csv.part",                  // 代次不是 20 位
        "11.2026101712345612345x.csv.part", // 代次里有非数字
        "12.20261017123456123456.x.csv.part", // 多一段
    ] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    let mut got: Vec<i64> = part_files(&dir).unwrap().into_iter().map(|(id, _)| id).collect();
    got.sort_unstable();
    assert_eq!(got, vec![7, 8], "只认两种导出半份名字");
    assert!(part_files(&dir.join("nope")).unwrap().is_empty(), "目录不存在 = 空手而归");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 半份文件按代次隔离：同一行两次认领必须落到两个不同的 `.part` 路径。
/// （否则旧执行者复活后会和新认领者截断/删除同一个文件。）
#[test]
fn part_path_is_generation_scoped() {
    let dir = Path::new("/tmp/exports");
    let a = part_path(dir, 42, now());
    let b = part_path(dir, 42, now() + chrono::Duration::microseconds(1));
    assert_ne!(a, b, "不同代次不能共用同一个 .part");
    let name = a.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("42.") && name.ends_with(".csv.part"), "命名约定: {name}");
    // 造出来的名字必须能被回收器认回来
    let tmp = std::env::temp_dir().join(format!("bee-part-gen-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join(name), b"x").unwrap();
    assert_eq!(part_files(&tmp).unwrap(), vec![(42, tmp.join(name))], "回收器认新名字");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// 成品也按代次隔离（N1）：同一行两次认领必须落到两个不同的 `.csv` 路径，
/// 否则输家的改名会盖掉赢家的成品。
#[test]
fn final_path_is_generation_scoped() {
    let dir = Path::new("/tmp/exports");
    let a = final_path(dir, 42, now());
    let b = final_path(dir, 42, now() + chrono::Duration::microseconds(1));
    assert_ne!(a, b, "不同代次不能共用同一个成品路径");
    let name = a.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("42.") && name.ends_with(".csv"), "命名约定: {name}");
    assert!(!name.ends_with(".part"), "成品名不能带 .part");
    // 造出来的名字必须能被清理器认回来
    let tmp = std::env::temp_dir().join(format!("bee-final-gen-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join(name), b"x").unwrap();
    assert_eq!(final_files(&tmp, 42).unwrap(), vec![tmp.join(name)], "清理器认新名字");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// 成品回收的名字约定：旧命名 `{id}.csv`（升级前）与新命名 `{id}.{20 位代次}.csv`
/// 都要认；半份、别人的 id、代次位数/字符不对的一律不看；目录不存在 = 空手而归。
#[test]
fn final_files_matches_old_and_generation_names() {
    let dir = std::env::temp_dir().join(format!("bee-finals-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in [
        "42.csv",                            // 旧命名（升级前的成品）
        "42.20261017123456123456.csv",       // 新命名
        "42.20261017123456123457.csv",       // 同一行的另一个代次（输家）
        "42.20261017123456123456.csv.part",  // 半份：另有回收路径
        "42.2026101712345612345x.csv",       // 代次里有非数字
        "42.123.csv",                        // 代次不是 20 位
        "142.csv",                           // 别的 id
        "43.csv",                            // 别的 id
        "42.csv.bak",                        // 运维手放的
    ] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    let mut got: Vec<String> = final_files(&dir, 42)
        .unwrap()
        .into_iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    got.sort_unstable();
    assert_eq!(
        got,
        vec!["42.20261017123456123456.csv", "42.20261017123456123457.csv", "42.csv"],
        "只认旧命名与代次命名的成品"
    );
    assert!(final_files(&dir.join("nope"), 42).unwrap().is_empty(), "目录不存在 = 空手而归");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 进程级闸只该有一个：两次 `gate()` 拿到同一个 Arc（旧实现每次 dispatch 新建 →
/// 跨 tick 叠加）。这条红了说明并发上限只能管住单次派发。
#[test]
fn gate_is_process_wide() {
    assert!(Arc::ptr_eq(&gate(), &gate()), "所有 dispatch 必须共享同一个进程级信号量");
}

/// 终态没写进去时撤不撤成品（M1）：只有**取消赢**（4）或行被删才撤；
/// 写库失败 / 被新执行者接管（行还是 1）/ 赢家已写 2、3 都必须留给赢家 ——
/// 否则会出现「status=2 却没有文件」。
#[test]
fn lost_finish_keeps_artifact_unless_canceled_or_deleted() {
    let r = |status: i8| {
        let mut t = sample_task();
        t.status = status;
        t
    };
    assert!(should_remove_artifact(None), "行没了：成品是孤儿，撤");
    assert!(should_remove_artifact(Some(&r(4))), "取消赢：不留成品");
    for status in [0, 1, 2, 3] {
        assert!(
            !should_remove_artifact(Some(&r(status))),
            "status={status}：不是取消赢，成品留给行当前的归属"
        );
    }
}

/// 认领谓词必须**排他**（I1）：老写法 `status IN (0, 1)` 在第一次把行推成 1 之后
/// 第二个执行者照样命中（同一任务两个执行者：双写 .part、双 finish）。
#[test]
fn claim_predicates_are_exclusive() {
    let pending = claim_sql(0);
    assert!(pending.ends_with("AND status = 0"), "待处理分支按 status = 0: {pending}");
    let stale = claim_sql(1);
    assert!(
        stale.ends_with("AND status = 1 AND started_at < ?"),
        "超时分支要带时戳条件（认领会刷 started_at，第二次不命中）: {stale}"
    );
    for sql in [pending, stale] {
        assert!(!sql.contains("IN (0, 1)"), "老谓词会让认领变成非排他: {sql}");
    }
}

/// 表头必须是**同步导出那份**（同一个函数），否则异步文件与同步导出会悄悄漂移。
#[test]
fn headers_come_from_the_sync_exporters() {
    assert_eq!((spec("audit-log").unwrap().headers)(), audit_log::headers());
    assert_eq!((spec("login-log").unwrap().headers)(), login_log::headers());
    assert!(audit_log::headers()[0] == "ID" && login_log::headers()[0] == "ID");
}

/// 认领 / 进度心跳 / 终态这三处全是**手写 SQL 列清单**（没有 ORM 兜底，见 `claim_sql`
/// 上面的注释）：加列（进度、心跳）必须三处同步，漏一处就是「这列永远是空/0」这种
/// 不报错的静默 bug。这里把三处钉在一起。
#[test]
fn claim_beat_and_finish_sql_agree_on_progress_and_heartbeat() {
    let pending = claim_sql(0);
    assert!(pending.contains("heartbeat_at = ?"), "认领那一手就要起心跳: {pending}");
    assert!(pending.contains("progress_rows = 0"), "重跑要清掉上一位执行者的进度: {pending}");

    let beat = beat_sql();
    assert!(beat.contains("progress_rows = ?") && beat.contains("heartbeat_at = ?"), "一次写回两列: {beat}");
    // 谓词带 status = 1 + 代次 token：取消（4）、被删、被新执行者重新认领的行都不许再写回
    assert!(
        beat.ends_with("AND status = 1 AND started_at = ?"),
        "心跳必须带归属 fencing（旧执行者复活后写不进）: {beat}"
    );

    let done = finish_sql();
    assert!(done.contains("progress_rows = ?"), "终态要把进度补齐（取消的行停在部分值）: {done}");
    assert!(done.contains("file_path = ?"), "路径与终态同一次写回: {done}");
    assert!(
        done.ends_with("AND status = 1 AND started_at = ?"),
        "取消赢 + 归属 fencing：终态不许覆盖 4，也不许覆盖赢家: {done}"
    );
}

/// 失联判定看心跳：跑得久但心跳还在的活任务不算死（M1 的正题），没见过心跳才回落
/// 到认领时刻（老行 / 认领后第一批还没写完就被 SIGKILL 的行）。
#[test]
fn last_seen_prefers_heartbeat_over_started_at() {
    let old = now() - chrono::Duration::seconds(3600);
    let fresh = now();
    let mut t = sample_task();
    t.started_at = Some(old);
    t.heartbeat_at = None;
    assert_eq!(t.last_seen(), Some(old), "没心跳过：回落认领时刻");
    t.heartbeat_at = Some(fresh);
    assert_eq!(t.last_seen(), Some(fresh), "有心跳：心跳说了算（跑了 1 小时也不算死）");
}

/// 残留 `.part` 收不收（批次 E T5 的口径 + 取消）：只看行的现状。
/// 取消的行（4）是**新加的一条**：它不会再有人跑，半份必须能被收 —— 漏了这条，
/// 取消 + SIGKILL 之后就永远留着孤儿文件（衔接 1）。
#[test]
fn collectable_covers_canceled_but_not_live_rows() {
    let (now_, stale) = (now(), now() - chrono::Duration::seconds(STALE_CLAIM_SECS));
    // 阈值是**严格**「早于」（认领/心跳都写 `< stale`）：正好卡在阈值那一秒的行还不算死。
    // 造「已卡死」的样本要用 dead（比阈值再早一秒），别拿 stale 当卡死值。
    let dead = stale - chrono::Duration::seconds(1);
    let r = |status: i8, last_seen: Option<NaiveDateTime>, expire_days: i64| {
        let mut t = sample_task();
        t.status = status;
        t.started_at = last_seen;
        t.heartbeat_at = None;
        t.expire_at = now() + chrono::Duration::days(expire_days);
        t
    };
    assert!(collectable(None, now_, stale), "行没了（运行中被删）= 孤儿");
    assert!(collectable(Some(&r(2, None, -1)), now_, stale), "已完成还留着半份");
    assert!(collectable(Some(&r(3, None, -1)), now_, stale), "已失败还留着半份");
    assert!(collectable(Some(&r(4, Some(dead), -1)), now_, stale), "已取消：半份要能收");
    assert!(!collectable(Some(&r(0, None, -1)), now_, stale), "待处理：马上要跑，别拆它的台");
    // 执行中：过期 + 超时（没心跳）才收；心跳还在的活任务一律不碰（M1）
    assert!(collectable(Some(&r(1, Some(dead), -1)), now_, stale), "卡死的执行中行");
    assert!(!collectable(Some(&r(1, Some(stale), -1)), now_, stale), "正好卡在阈值那一秒：还不算死（严格小于）");
    assert!(!collectable(Some(&r(1, Some(now_), -1)), now_, stale), "心跳还热的活任务");
    assert!(!collectable(Some(&r(1, Some(dead), 1)), now_, stale), "没到过期时刻不介入");
}

/// 并发闸：任务数超过上限时排队，且**每个任务都要跑到**；两次派发共享同一个闸
/// 时也不许叠加。导出任务在真库里是毫秒级的，集成测试观测不到重叠，所以「上限真的
/// 生效」只能在这里钉。
///
/// I1 的回归就在第二个场景：第一次 tick 派 4 个（还有 2 个在跑）、第二次 tick 再派 4 个
/// （待处理的行没变，仍会被扫到）—— 旧实现每次 `dispatch` 新建一个信号量，两批各跑
/// 各的，峰值会到 4；共享闸后峰值只能是 2。负控：额度 1 时峰值必须是 1。
#[tokio::test]
async fn dispatch_caps_concurrency_across_calls() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let peak = Arc::new(AtomicUsize::new(0));
    let live = Arc::new(AtomicUsize::new(0));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(16);
    let jobs = |n: usize| -> Vec<_> {
        (0..n)
            .map(|_| {
                let (peak, live, tx) = (peak.clone(), live.clone(), tx.clone());
                move || {
                    let (peak, live, tx) = (peak.clone(), live.clone(), tx.clone());
                    async move {
                        let cur = live.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(cur, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        live.fetch_sub(1, Ordering::SeqCst);
                        let _ = tx.send(()).await;
                    }
                }
            })
            .collect()
    };
    // 派发是「扔出去就返回」（不占住 job 循环），所以完成与否只能靠任务自己发消息回来数；
    // 带超时：漏跑的会让这里红在一条清楚的断言上，而不是把测试挂死
    async fn drain(rx: &mut tokio::sync::mpsc::Receiver<()>, n: usize) {
        for i in 0..n {
            tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .unwrap_or_else(|_| panic!("只有 {i} 个任务跑到头（应 {n} 个）"))
                .expect("发送端不会提前关闭");
        }
    }
    // 同一个闸 + 两次派发（= 两次 tick）交错：4+4 个任务抢 2 个额度
    let shared = Arc::new(tokio::sync::Semaphore::new(2));
    dispatch(shared.clone(), jobs(4));
    dispatch(shared.clone(), jobs(4));
    drain(&mut rx, 8).await;
    assert_eq!(
        peak.load(Ordering::SeqCst),
        2,
        "两次派发必须共享同一个闸：峰值超过 2 就是旧实现的叠加 bug"
    );
    // 负控：闸只有 1 个额度时峰值必须 1
    peak.store(0, Ordering::SeqCst);
    let one = Arc::new(tokio::sync::Semaphore::new(1));
    dispatch(one.clone(), jobs(3));
    drain(&mut rx, 3).await;
    assert_eq!(peak.load(Ordering::SeqCst), 1, "额度 1 时不许有并发");
}

/// 上面几条测试造行用的样板（字段值本身不参与断言，除了被单测改的那几列）。
fn sample_task() -> ExportTask {
    ExportTask {
        id: 1,
        admin_id: 1,
        username: "u".into(),
        kind: "audit-log".into(),
        params: "{}".into(),
        status: 0,
        file_path: String::new(),
        row_count: 0,
        file_size: 0,
        progress_rows: 0,
        msg: String::new(),
        created_at: now(),
        started_at: None,
        heartbeat_at: None,
        finished_at: None,
        expire_at: now(),
    }
}

/// 发起时校验 params 用的结构与取数用的一致：类型不对当场拒。
#[test]
fn check_rejects_wrong_param_types() {
    let k = spec("login-log").unwrap();
    assert!((k.check)(&serde_json::json!({})).is_ok(), "空筛选 = 全量，合法");
    assert!((k.check)(&serde_json::json!({"username": "a"})).is_ok());
    assert!((k.check)(&serde_json::json!({"status": "不是整数"})).is_err());
}
