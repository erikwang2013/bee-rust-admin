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
use crate::models::{Admin, Attachment, Job, JobLog};
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
    // 附件孤儿清理：每小时一趟 —— 配额是**行级**口径（删行立刻还配额），磁盘回收只靠它，
    // 收早也不怕：保护期 300s（ATTACH_MIN_AGE_SECS）远小于一小时，不会误收「先落盘后入库」的成品
    JobSpec {
        code: "attach_orphan_clean",
        name: "附件孤儿清理",
        interval_secs: 3600,
        run: |s| Box::pin(attach_orphan_clean(s)),
    },
    // 每天一份、保留 7 份（[backup] keep）：默认节奏是一周内的回滚窗口。
    // 备份本体在 crate::backup（解析 DSN → mysqldump → 落盘 → 清理）
    JobSpec {
        code: "db_backup",
        name: "数据库备份",
        interval_secs: 86400,
        run: |s| Box::pin(crate::backup::run(s)),
    },
    // 异步导出（项 4）：执行体在 crate::export。每分钟一趟，发起后最迟一分钟跑完；
    // 单独一个清理任务（每天）删过期文件与记录
    JobSpec {
        code: "export_run",
        name: "导出任务执行",
        interval_secs: 60,
        run: |s| Box::pin(crate::export::run(s)),
    },
    JobSpec {
        code: "export_clean",
        name: "导出文件清理",
        interval_secs: 86400,
        run: |s| Box::pin(crate::export::clean(s)),
    },
    // 登录设备清理（E2-2）：清「已踢且 [auth] device_keep_days 天没露面」的行，
    // 执行体在 crate::retention（口径与安全线写在那边）
    JobSpec {
        code: "device_clean",
        name: "登录设备清理",
        interval_secs: 86400,
        run: |s| Box::pin(crate::retention::device_clean(s)),
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

/// 同进程内的「已在跑」闸：手动触发改成**后台执行**后（api::job::run 里 spawn），
/// 光靠 `last_run_at` 拦不住并发 —— 它只在调度 tick 内部串行有效，连点几次触发就是
/// 几份同时跑（db_backup 叠出多份 mysqldump）。粒度是 code：同一个任务不并发，
/// 不同任务互不影响。shortcut: 只在单进程内生效（与调度同一前提），多实例部署要换成 DB 行锁。
static RUNNING: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

/// 占位凭据：执行全程持有，Drop 时释放 —— 正常返回、提前 return、panic 都不会漏放。
/// 手动触发的接口在 **spawn 之前** 调 [`Busy::acquire`]（失败当场 409），把 permit
/// 交给 [`run_holding`]；调度循环走 [`run`]（内部自己拿）。
pub struct Busy(&'static str);
impl Busy {
    pub fn acquire(code: &'static str) -> Option<Self> {
        let mut v = RUNNING.lock().ok()?;
        if v.contains(&code) {
            return None;
        }
        v.push(code);
        Some(Self(code))
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        if let Ok(mut v) = RUNNING.lock() {
            v.retain(|c| *c != self.0);
        }
    }
}

/// 调度循环的入口：自己拿闸，拿不到就跳过（不写 job_log、不动 `last_run_at`：
/// 没执行就不是一次执行记录）。
pub async fn run(state: &AppState, job: Job, spec: &JobSpec) -> (i8, String, i64) {
    let Some(permit) = Busy::acquire(spec.code) else {
        return (0, format!("跳过：任务 {} 上一次还在执行中", spec.code), 0);
    };
    run_holding(state, job, spec, permit).await
}

/// 执行一个任务（**调用方已持 permit**）：抢占 → 跑 → 写 `job_log` → 回写 job 行。
/// 手动触发在接口层先 [`Busy::acquire`]，拿到才 spawn 到这里 —— 这样「受理」与
/// 「占住闸」是同一个同步动作，不存在「接口回 200、任务体却静默跳过」的窗口。
/// permit 随任务走到函数结束（Drop 释放）。
pub async fn run_holding(
    state: &AppState,
    job: Job,
    spec: &JobSpec,
    _permit: Busy,
) -> (i8, String, i64) {
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
/// - 列里是插件的 `saved_path`（`avatar_202610_<md5>.png`）→ 按上传根解析成相对路径；
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
            // saved_path → 插件的成品路径；只保留在本目录下的（配置换了 upload_dir 的
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
                purge_instant(&state, crate::api::avatar::AVATAR_GROUP, rel);
            }
            Err(e) => tracing::warn!("删孤儿头像 {} 失败: {e}", path.display()),
        }
    }
    Ok(format!("头像 {seen} 个，删除孤儿 {removed} 个"))
}

/// 附件孤儿清理：删附件组目录（`{upload_dir}/attach/`）下**不再被任何 `attachment` 行
/// 引用**的文件。护栏与头像同一条：只删白名单扩展名里的文件 —— 目录里除了成品还有
/// 插件写到一半的临时文件（`<临时名>.png.part`）与运维手放的东西，那些不碰。
///
/// 与头像的差别：附件的引用集合只有一个来源（`attachment.stored_path`，没有历史 URL 形态），
/// 所以这里简单得多。上传成品落盘成功、INSERT 失败的那条缝（以及被人手删的行）只有它兜得住。
async fn attach_orphan_clean(state: AppState) -> Result<String, String> {
    let dir = std::path::Path::new(&state.cfg.upload_dir).join(crate::api::attach::ATTACH_GROUP);
    let files = match collect_files(&dir) {
        Ok(files) => files,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok("附件目录不存在，无需清理".into());
        }
        Err(e) => return Err(format!("读附件目录 {} 失败: {e}", dir.display())),
    };

    let rows = Attachment::query()
        .all(&state.db)
        .await
        .map_err(|e| format!("查附件失败: {e}"))?;
    let mut kept: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    for row in &rows {
        let stored = row.stored_path.trim();
        if stored.is_empty() {
            continue;
        }
        // saved_path → 插件的成品路径；配置换了 upload_dir 的陈旧值解析到别处，那不是本目录的孤儿
        if let Some(resource) = state.aether.resource(stored) {
            if let Ok(rel) = resource.path.strip_prefix(&dir) {
                kept.insert(rel.to_path_buf());
            }
        }
    }

    let (mut seen, mut removed, mut fresh) = (0u64, 0u64, 0u64);
    for path in files {
        seen += 1;
        if !is_attach_file(&path) {
            continue;
        }
        let Ok(rel) = path.strip_prefix(&dir) else { continue };
        if kept.contains(rel) {
            continue;
        }
        // 保护期（批次 E S2）：刚写下的文件先不碰 —— 上传是「先落盘、后 INSERT」，
        // 清理撞进这个窗口就会删掉一份马上要落库的成品（也顺带让「删行不删文件」的
        // 延迟回收有个确定的下限：文件至少活到下一轮清理）。
        if too_fresh(&path) {
            fresh += 1;
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(_) => {
                removed += 1;
                purge_instant(&state, crate::api::attach::ATTACH_GROUP, rel);
            }
            Err(e) => tracing::warn!("删孤儿附件 {} 失败: {e}", path.display()),
        }
    }
    Ok(format!("附件 {seen} 个，删除孤儿 {removed} 个，保护期内跳过 {fresh} 个"))
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
/// 索引里可能还留着 `group_hash → saved_path` 指向这个已删文件（管理员行被删、值被直接
/// 改库 —— 文件成了孤儿但索引没过期），不清的话下次有人传同样的内容会「秒传命中」到一个
/// 死链（头像 404）。`rel` 是相对**该分组目录**的路径，只有「一层子目录 + 文件名」这种
/// 插件成品的位置才构造得出 saved_path；直接躺在组目录下的老文件/手工文件跳过。
fn purge_instant(state: &AppState, group: &str, rel: &std::path::Path) {
    let (Some(subdir), Some(name)) = (
        rel.parent().and_then(|p| p.to_str()),
        rel.file_name().and_then(|n| n.to_str()),
    ) else {
        return;
    };
    if subdir.is_empty() || subdir.contains('/') {
        return;
    }
    let saved = aetherupload::SavedPath::encode(group, subdir, name);
    if !state.aether.delete_instant_path(&saved) {
        tracing::debug!("清孤儿 {saved} 的秒传索引未成功（不阻断清理）");
    }
}

/// 扩展名白名单守卫（理由见 `avatar_orphan_clean` 的注释）：只有 png/jpg 才可能是
/// 成品头像；`.part`、`.bak`、无扩展名的一律留给人工处理。
fn is_avatar_file(path: &std::path::Path) -> bool {
    in_ext_whitelist(path, &crate::api::avatar::AVATAR_EXTS)
}

/// 附件同理：只有 `ATTACH_EXTS` 里的扩展名才可能是上传成品。
fn is_attach_file(path: &std::path::Path) -> bool {
    in_ext_whitelist(path, &crate::api::attach::ATTACH_EXTS)
}

/// 孤儿文件的保护期（批次 E S2）。上传是「先落盘、后 INSERT」，两者之间有毫秒级窗口；
/// 附件删除也改成「只删行、文件交给清理」（延迟回收）—— 有这条下限，刚写下的文件不会被
/// 任何一轮清理误收。5 分钟：比任何一次上传耗时都长得多，又短到磁盘不会被垃圾拖住。
const ATTACH_MIN_AGE_SECS: u64 = 300;

/// 文件「太新」= 保护期内，清理不碰。读不到 mtime（个别文件系统 / 文件刚被并发删掉）
/// 一律按太新处理：宁可晚一轮收（最多多留一天），不可删错。
fn too_fresh(path: &std::path::Path) -> bool {
    match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(mtime) => mtime
            .elapsed()
            .map(|age| age.as_secs() < ATTACH_MIN_AGE_SECS)
            // mtime 在「未来」（时钟回拨/漂移）：同样当太新，等下一轮
            .unwrap_or(true),
        Err(_) => true,
    }
}

fn in_ext_whitelist(path: &std::path::Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| exts.contains(&ext.as_str()))
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
}
