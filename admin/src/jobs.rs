// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 定时任务（v1.6.0 C2a）：任务体**代码注册**（`JOBS`），数据库只管调度与执行记录。
//!
//! 调度语义：
//! - `job.cron` 存的是**间隔秒数**（列名保留 cron 是为了将来换表达式时不改列名）；
//!   不做 cron 表达式解析：内置任务都是「每小时/每天跑一次」，间隔语义足够。
//! - 循环每 10 秒 tick 一次，选出 `status=1` 且 `last_run_at` 为空或已过间隔的任务
//!   **串行**执行（任务少，并发带来的写冲突不值得）。
//! - 同一进程内用 `last_run_at` **抢占**：执行前先落库，tick 重叠时不会再选它。
//!   ponytail: 单实例假设，多实例部署要换成 DB 行锁（本项目的 admin 是单实例）。
//! - `[job] enabled=false` 时**不启动循环**（手动触发仍可用：运维要能临时关掉定时、手动补跑）。
use crate::models::{Admin, Job, JobLog};
use crate::state::AppState;
use bee_orm::Model;
use crate::util::now;
use std::time::Duration;

type JobFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>;

/// 一条代码注册的任务。`run` 收 `AppState`（内部都是 Arc，克隆廉价）。
pub struct JobSpec {
    pub code: &'static str,
    pub name: &'static str,
    /// 首次 seed 写进 `job.cron` 的默认间隔秒数；之后以库里的值为准
    pub interval_secs: i64,
    /// 返回的字符串进 `job_log.msg`
    pub run: fn(AppState) -> JobFuture,
}

pub const JOBS: &[JobSpec] = &[
    JobSpec {
        code: "log_retention",
        name: "日志清理",
        interval_secs: 86400,
        run: |s| Box::pin(log_retention(s)),
    },
    JobSpec {
        code: "avatar_orphan_clean",
        name: "头像孤儿清理",
        interval_secs: 86400,
        run: |s| Box::pin(avatar_orphan_clean(s)),
    },
];

/// 按 code 找注册项；库里残留的旧 code 找不到 = 不可手动触发（409）、调度跳过。
pub fn spec(code: &str) -> Option<&'static JobSpec> {
    JOBS.iter().find(|j| j.code == code)
}

/// 调度循环的 tick 间隔。
const TICK: Duration = Duration::from_secs(10);

/// 调度循环：每 10 秒选一次到期任务串行跑。由 main 在 `[job] enabled` 时 spawn。
pub async fn scheduler(state: AppState) {
    let mut tick = tokio::time::interval(TICK);
    // 任务跑超一个 tick 时不要补跑积压的 tick（默认 Burst 会连着触发一串）
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        tick_once(&state).await;
    }
}

/// 一个 tick：把到期的任务串行跑一遍。查库失败只告警，下一个 tick 再试。
async fn tick_once(state: &AppState) {
    let qs = match Job::query().filter_eq("status", 1) {
        Ok(qs) => qs,
        Err(e) => return tracing::warn!("构造任务查询失败: {e}"),
    };
    let jobs = match qs.all(&state.db).await {
        Ok(v) => v,
        Err(e) => return tracing::warn!("查询到期任务失败: {e}"),
    };
    for job in jobs {
        if !due(&job, now()) {
            continue;
        }
        // 库里残留的旧 code：跳过（不报错——它只是不会再被执行，列表里仍看得到）
        let Some(spec) = spec(&job.code) else {
            tracing::debug!("任务 {} 不在代码注册表里，跳过", job.code);
            continue;
        };
        run(state, job, spec).await;
    }
}

/// `cron`（间隔秒数）→ 秒数。0 / 非数字（手工改库改坏）→ None = 永不到期。
fn interval_secs(cron: &str) -> Option<i64> {
    cron.trim().parse::<i64>().ok().filter(|s| *s > 0)
}

/// 到期判定：没跑过（NULL）= 到期；否则 `now - last_run_at >= 间隔`。
fn due(job: &Job, now: chrono::NaiveDateTime) -> bool {
    let Some(interval) = interval_secs(&job.cron) else {
        return false;
    };
    match job.last_run_at {
        None => true,
        Some(last) => now >= last + chrono::Duration::seconds(interval as i64),
    }
}

/// 执行一个任务：抢占 → 跑 → 写 `job_log` → 回写 job 行的执行结果。
/// 手动触发与调度循环都走这里（`last_run_at` 两边都更新：UI 的「上次运行」要如实）。
pub async fn run(state: &AppState, job: Job, spec: &JobSpec) -> (i8, String, i64) {
    let started = now();
    // 抢占：先把 last_run_at 落库，同一进程的下一 tick 不会再选它
    let mut claimed = job.clone();
    claimed.last_run_at = Some(started);
    claimed.updated_at = started;
    if let Err(e) = claimed.update(&state.db).await {
        tracing::warn!("抢占任务 {} 失败: {e}", job.code);
    }

    let t0 = std::time::Instant::now();
    let (status, msg) = match (spec.run)(state.clone()).await {
        Ok(m) => (1i8, m),
        Err(e) => (0i8, e),
    };
    let duration_ms = t0.elapsed().as_millis() as i64;
    // 手工摘要/错误信息可能带换行或超长：列是 varchar(255)
    let msg: String = msg.replace(['\r', '\n'], " ").chars().take(255).collect();

    // 记录写失败不能影响任务本身（发号失败也一样：跳过这条记录）
    match state.next_id() {
        Ok(id) => {
            let row = JobLog {
                id,
                job_code: job.code.clone(),
                started_at: started,
                duration_ms,
                status,
                msg: msg.clone(),
            };
            if let Err(e) = row.insert(&state.db).await {
                tracing::warn!("写任务执行记录失败: {e}");
            }
        }
        Err(e) => tracing::warn!("生成任务记录 id 失败: {e:?}"),
    }

    let mut done = claimed;
    done.last_status = Some(status);
    done.last_msg = msg.clone();
    done.updated_at = now();
    if let Err(e) = done.update(&state.db).await {
        tracing::warn!("回写任务 {} 状态失败: {e}", job.code);
    }
    (status, msg, duration_ms)
}

/// 日志保留：清 `login_log` / `audit_log` / `job_log` 里超过 `[log] retain_days` 的行。
/// 沿用 A4 的实现；`retain_days = 0`（永久保留）时什么都不做。
async fn log_retention(state: AppState) -> Result<String, String> {
    let days = state.cfg.retain_days;
    if days <= 0 {
        return Ok("retain_days=0（永久保留），跳过".into());
    }
    let n = crate::retention::purge_all(&state.db, days).await;
    Ok(format!("清理过期日志 {n} 条（保留 {days} 天）"))
}

/// 头像孤儿清理：删头像组目录（`{upload_dir}/avatar/`）下**不再被任何 admin 引用**的
/// 头像文件。引用形态两种都认（4b 前后各一种）：
/// - 列里是插件的 `savedPath`（`avatar_202610_<md5>.png`）→ 按上传根解析成相对路径；
/// - 列里是老 URL（`/api/v1/avatar/xxx`）→ 老落盘路径是 `{id}.png` / `{id}.jpg`，
///   两种扩展名都算引用 —— 存量头像没迁移前仍在用，误删就是把别人的头像删没了。
///
/// **护栏：只删扩展名在 png/jpg 白名单里的文件，其他一律不碰。**
/// 看起来像可以删掉的啰嗦，其实是在防误删 —— 目录里除了成品头像还有：
/// 插件写到一半的临时文件（`<临时名>.png.part`，扩展名是 `part`）、运维/用户手放的
/// 文件（备份、说明、别人家的上传）。这些没有引用关系可依赖，只能靠白名单划界。
/// 反过来说：白名单内、又没人引用的 png/jpg 就是孤儿（包括用户手放的），该删。
/// 留着的代价只是占点磁盘，删错的代价是丢数据 —— 划界就按这条。
async fn avatar_orphan_clean(state: AppState) -> Result<String, String> {
    let dir = std::path::Path::new(&state.cfg.upload_dir).join("avatar");
    let files = match collect_files(&dir) {
        Ok(files) => files,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok("头像目录不存在，无需清理".into());
        }
        Err(e) => return Err(format!("读头像目录 {} 失败: {e}", dir.display())),
    };

    // 引用集合：相对头像组目录的路径（与下面 walk 出来的相对路径同一坐标系）
    let admins = Admin::query()
        .all(&state.db)
        .await
        .map_err(|e| format!("查管理员失败: {e}"))?;
    let mut kept: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    for admin in &admins {
        let stored = admin.avatar.trim();
        if stored.is_empty() {
            continue;
        }
        if stored.starts_with('/') {
            // 老 URL：落盘在老路径上，两种扩展名都算引用（不确定哪个存在，两个都留着）
            for ext in crate::api::avatar::AVATAR_EXTS {
                kept.insert(std::path::PathBuf::from(format!("{}.{ext}", admin.id)));
            }
        } else if let Some(resource) = state.aether.resource(stored) {
            // savedPath → 插件的成品路径；只保留在本目录下的（配置换了 upload_dir 的
            // 陈旧值解析到别处，那不是本目录的孤儿，这里不碰）
            if let Ok(rel) = resource.path.strip_prefix(&dir) {
                kept.insert(rel.to_path_buf());
            }
        }
    }

    let (mut seen, mut removed) = (0u64, 0u64);
    for path in files {
        seen += 1;
        if !is_avatar_file(&path) {
            continue;
        }
        let Ok(rel) = path.strip_prefix(&dir) else { continue };
        if kept.contains(rel) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(_) => {
                removed += 1;
                purge_instant(&state, rel);
            }
            Err(e) => tracing::warn!("删孤儿头像 {} 失败: {e}", path.display()),
        }
    }
    Ok(format!("头像 {seen} 个，删除孤儿 {removed} 个"))
}

/// 递归收集目录下的所有文件（用显式栈：用户手放的深层目录不该让清理任务爆栈）。
/// 子目录读失败只告警跳过 —— 清理任务是卫生工作，不能因为一个权限问题整轮失败。
fn collect_files(root: &std::path::Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if dir == root => return Err(e),
            Err(e) => {
                tracing::warn!("读头像子目录 {} 失败: {e}", dir.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// 删掉成品文件后，把它的秒传索引一起清掉。
///
/// 索引里可能还留着 `group_hash → savedPath` 指向这个已删文件（管理员行被删、值被直接
/// 改库 —— 文件成了孤儿但索引没过期），不清的话下次有人传同样的内容会「秒传命中」到一个
/// 死链（头像 404）。`rel` 是相对头像组目录的路径，只有「一层子目录 + 文件名」这种插件
/// 成品的位置才构造得出 savedPath；直接躺在 `avatar/` 下的老文件/手工文件跳过。
fn purge_instant(state: &AppState, rel: &std::path::Path) {
    let (Some(subdir), Some(name)) = (
        rel.parent().and_then(|p| p.to_str()),
        rel.file_name().and_then(|n| n.to_str()),
    ) else {
        return;
    };
    if subdir.is_empty() || subdir.contains('/') {
        return;
    }
    let saved = aetherupload::SavedPath::encode(crate::api::avatar::AVATAR_GROUP, subdir, name);
    if !state.aether.delete_instant_path(&saved) {
        tracing::debug!("清孤儿 {saved} 的秒传索引未成功（不阻断清理）");
    }
}

/// 扩展名白名单守卫（理由见 `avatar_orphan_clean` 的注释）：只有 png/jpg 才可能是
/// 成品头像；`.part`、`.bak`、无扩展名的一律留给人工处理。
fn is_avatar_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| crate::api::avatar::AVATAR_EXTS.contains(&ext.as_str()))
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn registry_codes_are_unique_and_looked_up() {
        assert!(spec("log_retention").is_some());
        assert!(spec("avatar_orphan_clean").is_some());
        assert!(spec("ghost_job").is_none(), "库里残留的旧 code 找不到注册项");
        let mut codes: Vec<&str> = JOBS.iter().map(|j| j.code).collect();
        codes.sort_unstable();
        let n = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), n, "注册表里的 code 不能重复");
    }
}
