// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 异步导出（项 4）：导出**类型**的代码注册表（`KINDS`）+ 执行/清理两个任务体。
//!
//! 口径：
//! - 每个 kind 的 CSV 表头与取数函数**直接复用同步导出的那份**（`api::audit_log` /
//!   `api::login_log` 里的 `headers()` / `export_batch()`）——同一个文件 `/xxx/export`
//!   与这里的异步文件逐字节一致，没有第二套渲染逻辑可变。
//! - 执行挂在既有 job 循环（`JOBS` 注册，样式与项 9 的 db_backup 一致）：
//!   `export_run` 每分钟把待处理任务跑掉，`export_clean` 每天清过期（记录 + 文件）。
//! - 数据权限在**执行时**按发起人**当前**的角色重算（`datascope::from_roles`），不是发起时
//!   快照：权限被收窄后，还没跑完的任务不会把已经看不到的数据导出去；发起人被删 → 任务失败。
//! - 文件落在 `{upload_dir}/exports/<任务 id>.csv`（先写 `{id}.{代次}.csv.part` 再改名，
//!   中断不留半份）。代次 = 认领时写的 `started_at`：同一行被重新认领后，新旧执行者的
//!   `.part` 互不覆盖，输家的 `PartFile` 也只会删自己那份。
//! - 任务可以**取消**（行置 4）：执行者每批回写进度时顺带探一次（谓词 `status = 1` 且
//!   `started_at = 本代次` 命中 0 行 = 被取消或被新执行者接管），批间收手、半份由
//!   `PartFile` 清掉；同一批 5000 行内不多停。收尾那一写也用同一把 fencing token
//!   （见 `finish`）：没写进去时**先读行的现状**——只有已取消（4）或行已被删才撤成品，
//!   写库失败/被接管都把文件留给赢家（M1：不能出现「status=2 却没有文件」）。
//! - 进度与**心跳**一起回写（`progress_rows` / `heartbeat_at`）：判「进程崩过」与收残留
//!   `.part` 都看心跳（`last_seen`），长任务因此不会被误判成死任务重跑（M1）。
//! - 任务是**并发派发**的（`tokio::spawn` + **进程级共享**信号量 `EXPORT_CONCURRENCY`），
//!   `run()` 派发完就返回，不占住 job 循环；跨 tick / 多次手动触发共用同一个闸，不会
//!   叠加出超过 2 个同时执行的任务；同一行仍由认领排他保证只有一个执行者。
use crate::api::{audit_log, csv, login_log};
use crate::datascope::{self, DataScope};
use crate::error::ApiError;
use crate::models::{Admin, ExportTask, Role};
use crate::state::AppState;
use crate::util::now;
use crate::relations::RelationsExt;
use bee_orm::Value;
use chrono::NaiveDateTime;
use serde_json::Value as JsonVal;
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

type BatchFuture = Pin<Box<dyn Future<Output = Result<Vec<(i64, String)>, ApiError>> + Send>>;

/// 一种可导出的数据。新增 kind = 在 `KINDS` 里加一条 + 前端加一个 i18n 名字，
/// 不加权限码（复用该模块的 `:list`：看得到才导得出）。
pub struct Kind {
    pub code: &'static str,
    /// 发起与下载都要过的查看码（= 该模块列表页的 `:list`）
    pub perm: &'static str,
    /// 落盘/下载文件名前缀
    pub file_prefix: &'static str,
    /// CSV 表头（与同步导出共用同一函数）
    pub headers: fn() -> Vec<String>,
    /// 发起时校验 params：与取数用**同一个** Deserialize 结构，不出现两套口径
    pub check: fn(&JsonVal) -> Result<(), String>,
    /// 按 params 取一批（keyset：`last` 是上一批最后一行 id）
    pub batch: fn(AppState, JsonVal, DataScope, Option<i64>) -> BatchFuture,
}

pub const KINDS: &[Kind] = &[
    Kind {
        code: "audit-log",
        perm: "system:auditlog:list",
        file_prefix: "audit-logs",
        headers: audit_log::headers,
        check: |p| parse::<audit_log::AuditListQuery>(p),
        batch: |s, p, sc, last| {
            Box::pin(async move {
                let q: audit_log::AuditListQuery =
                    serde_json::from_value(p).map_err(|e| ApiError::BadRequest(format!("筛选参数不合法: {e}")))?;
                audit_log::export_batch(&s, &q, &sc, last).await
            })
        },
    },
    Kind {
        code: "login-log",
        perm: "system:loginlog:list",
        file_prefix: "login-logs",
        headers: login_log::headers,
        check: |p| parse::<login_log::LogListQuery>(p),
        batch: |s, p, sc, last| {
            Box::pin(async move {
                let q: login_log::LogListQuery =
                    serde_json::from_value(p).map_err(|e| ApiError::BadRequest(format!("筛选参数不合法: {e}")))?;
                login_log::export_batch(&s, &q, &sc, last).await
            })
        },
    },
];

/// 按 code 找注册项；库里残留的旧 kind（已经下线的类型）找不到 = 任务失败。
pub fn spec(code: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.code == code)
}

fn parse<T: serde::de::DeserializeOwned>(p: &JsonVal) -> Result<(), String> {
    serde_json::from_value::<T>(p.clone()).map(|_| ()).map_err(|e| e.to_string())
}

/// 导出文件目录：`{upload_dir}/exports`（不单开配置项——它跟着上传目录走就够）。
pub fn dir(state: &AppState) -> PathBuf {
    Path::new(&state.cfg.upload_dir).join("exports")
}

/// 认领超时 / 心跳超时：执行中的行超过这么久**没有心跳**（从没心跳过就看认领时刻），
/// 当成「进程崩过」重跑一次。心跳由每个 5000 行批回写（`beat`），只要还在跑就一直有 ——
/// 长任务不会再被误判成死任务重跑一遍（M1）。
/// shortcut: 固定 10 分钟，真要按任务类型分档再进配置（导出都是分钟级，够用）
const STALE_CLAIM_SECS: i64 = 600;

/// 进程内同时跑几个导出任务。取数批接口是 `LIMIT 5000` 的顺序读，开 2 条既能把
/// 「一个慢任务堵住别人」摊平，又不至于把库压垮（后台任务，慢一点没关系）。
/// 上限是**进程级**的：`gate()` 里那一个信号量由所有 `dispatch()` 调用共享 —— 跨 tick /
/// 多次手动触发也不会叠加（旧实现每次派发新建一个，两批各跑各的，实际能超过 2）。
/// 同一行只被一个执行者认领由 `claim` 的排他谓词保证，与本闸是两回事。
const EXPORT_CONCURRENCY: usize = 2;
const _: () = assert!(EXPORT_CONCURRENCY > 0, "并发闸给 0 会死等");

/// 进程级并发闸（与 job Busy 的进程内语义一致）：懒建一次，之后所有派发共用。
/// 只在本模块里取用，没人会 `close()` 它 —— `acquire_owned` 因此不会返回错误。
fn gate() -> Arc<tokio::sync::Semaphore> {
    static GATE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(EXPORT_CONCURRENCY))).clone()
}

/// `export_run`：把待处理（或卡住的）任务派发出去，每个任务自己抢并发额度。
/// **不再占住 job 循环**：派发完就返回，成败看任务行自己的状态 —— `job_log.msg` 因此
/// 只说派发了几个（原来那种「成功 N / 失败 M」要等所有任务跑完才能写，就又变成占用）。
pub async fn run(state: AppState) -> Result<String, String> {
    let pending = ExportTask::query()
        .filter_eq("status", 0)
        .map_err(|e| e.to_string())?
        .all(&state.db)
        .await
        .map_err(|e| format!("查待处理导出失败: {e}"))?;
    // 执行中的行里挑「疑似进程崩过」的：按**心跳**判。放 Rust 里比是因为它是
    // 「心跳优先、没有才看认领时刻」（`ExportTask::last_seen`），写成 SQL 要 COALESCE，
    // 口径就散到两处了；而执行中的行本来就没几条（并发上限 2 + 卡住的少数）
    let stale = now() - chrono::Duration::seconds(STALE_CLAIM_SECS);
    let stuck: Vec<ExportTask> = ExportTask::query()
        .filter_eq("status", 1)
        .map_err(|e| e.to_string())?
        .all(&state.db)
        .await
        .map_err(|e| format!("查执行中的导出失败: {e}"))?
        .into_iter()
        .filter(|t| t.last_seen().map_or(true, |s| s < stale))
        .collect();

    let tasks: Vec<ExportTask> = pending.into_iter().chain(stuck).collect();
    if tasks.is_empty() {
        return Ok("导出任务 0 个".to_string());
    }
    let n = tasks.len();
    let jobs = tasks
        .into_iter()
        .map(|task| {
            let st = state.clone();
            move || {
                let st = st.clone();
                async move { run_claimed(st, task).await }
            }
        })
        .collect();
    dispatch(gate(), jobs);
    Ok(format!("导出任务 {n} 个（已派发，并发上限 {EXPORT_CONCURRENCY}）"))
}

/// 并发闸 + 派发：把任务交给后台跑（闸上最多 `EXPORT_CONCURRENCY` 个同时在跑，其余排队
/// 等额度），**派发完就返回** —— 这是 E2-1 要的形态：一个跑几分钟的大导出不再占住 job
/// 循环（循环里还排着备份、清理那些任务）。终态写回、失败日志都在 `run_claimed` 内部，
/// 这里不需要结果；单个任务 panic 也只丢它自己（`tokio::spawn` 把 panic 挡在里面）。
/// 额度在**每个 spawn 出去的任务内部**才申请 —— 不是在派发循环里等（那样又串行了）。
/// `sem` 由调用方给（生产就是进程级 `gate()`）：两次派发传同一个闸，就会互相排队。
fn dispatch<F, Fut, T>(sem: Arc<tokio::sync::Semaphore>, jobs: Vec<F>)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    for job in jobs {
        let sem = sem.clone();
        tokio::spawn(async move {
            // permit 活到闭包结束：这个任务跑完（含出错返回）才把额度还回去
            let _permit = sem.acquire_owned().await.expect("并发闸只有本模块持有，没人会关");
            job().await
        });
    }
}

/// 拿到并发额度之后：认领 → 跑 → 写终态。认领不上就静默跳过（别的执行者先拿到，
/// 或行已经被取消/删掉）。
async fn run_claimed(state: AppState, task: ExportTask) {
    let seen = match claim(&state, &task).await {
        Ok(Some(seen)) => seen,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!("认领导出任务 {} 失败: {e}", task.id);
            return;
        }
    };
    match run_one(&state, &task, seen).await {
        Ok(RunEnd::Done) => {} // 终态已由 run_one 写好（收尾那一写带本代次 token）
        // 行已不归本执行者（取消 / 被新执行者接管 / 行被删）：这里**一个字段都不写**，
        // 写了就是把别人的状态盖回去。进度停在收手那一刻的部分值，留着给人看。
        Ok(RunEnd::Lost) => tracing::info!("导出任务 {} 的行已不归本执行者，收手", task.id),
        Err(e) => {
            tracing::warn!("导出任务 {} 失败: {e}", task.id);
            // 失败行也把 msg 落库：界面上要看得见原因，不然只能翻日志
            let msg: String = e.chars().take(255).collect();
            let _ = finish(&state, task.id, seen, 3, 0, 0, "", &msg).await;
        }
    }
}

/// 认领一条任务：把行推进「执行中」的那一次 UPDATE 算认领成功，别的执行者拿到 0 行。
/// 谓词必须**排他**：曾经写成 `status IN (0, 1)` 是错的 —— 第一次 UPDATE 把行改成 1 之后，
/// 第二个执行者用同样的条件照样命中（1 ∈ (0,1)），同一任务就有两个执行者（双写 .part、
/// 双 finish 互相覆盖）。所以按进入时的状态分两条：
/// - 待处理（status=0）：`status = 0` 只在第一次命中（认领后行变成 1）；
/// - 执行中超时（status=1 且久无心跳，见上面查 `stuck` 的口径）：认领会把 `started_at`
///   与 `heartbeat_at` **一起**刷成现在，第二个执行者的 `started_at < 超时线` 不再命中。
///   （挑行看心跳、认领仍看 `started_at`，两边不矛盾：认领一次把两个时戳同时刷新，
///   所以第二个执行者按哪个看，看到的值都变成「刚刚」。）
/// 认领成功时返回**本次写进 `started_at` 的时刻**：它是这一代执行者的 fencing token，
/// 后面 `beat` / `finish` 的 UPDATE 都带 `AND started_at = ?`。行被重新认领后 token 变掉，
/// 旧执行者从此写不进一个字段（活着的旧执行者超时复活也不会和新认领者同写）。
async fn claim(state: &AppState, task: &ExportTask) -> Result<Option<NaiveDateTime>, String> {
    let sql = claim_sql(task.status);
    let ts = now();
    let mut args = vec![Value::from(ts), Value::from(ts), Value::from(task.id)];
    if task.status != 0 {
        args.push(Value::from(ts - chrono::Duration::seconds(STALE_CLAIM_SECS)));
    }
    let n = state
        .db
        .execute(sql, &args)
        .await
        .map_err(|e| format!("认领导出任务 {} 失败: {e}", task.id))?;
    Ok((n > 0).then_some(ts))
}

/// 认领用的两条语句。单独成函数是为了能被**无库单测**钉住谓词：集成测试在另一个 crate
/// 里只能复制 SQL（bin crate 不能 use 自己的代码），复制品对 src 不可证伪（老谓词回退时
/// 它照样绿），所以谓词本身的排他性在这里钉。
///
/// 顺带起心跳、清进度：认领是这三列（status / started_at / heartbeat_at / progress_rows）
/// 唯一会一起写的地方，认领之后的行一定带着一位「还活着」的证据。
fn claim_sql(status: i8) -> &'static str {
    if status == 0 {
        "UPDATE export_task SET status = 1, started_at = ?, heartbeat_at = ?, progress_rows = 0 WHERE id = ? AND status = 0"
    } else {
        "UPDATE export_task SET status = 1, started_at = ?, heartbeat_at = ?, progress_rows = 0 WHERE id = ? AND status = 1 AND started_at < ?"
    }
}

/// 每批回写进度 + 心跳。一次 UPDATE 干三件事：报进度、发心跳、**探取消/探归属** ——
/// 谓词带 `status = 1` 与 `started_at = ?`（本代次 token）：被取消（4）、被删、或被新
/// 执行者重新认领（token 变了）的行都命中 0 行，0 行就是「别再干了」的信号，不额外
/// 多一次 SELECT。单独成函数同 `claim_sql`：谓词钉在单测里。
fn beat_sql() -> &'static str {
    "UPDATE export_task SET progress_rows = ?, heartbeat_at = ? WHERE id = ? AND status = 1 AND started_at = ?"
}

/// 回写进度 / 心跳；`false` = 这行已经不归我们了（被取消、被删或被接管），批间就该收手。
async fn beat(state: &AppState, id: i64, seen: NaiveDateTime, rows: i64) -> Result<bool, String> {
    let n = state
        .db
        .execute(
            beat_sql(),
            &[Value::from(rows), Value::from(now()), Value::from(id), Value::from(seen)],
        )
        .await
        .map_err(|e| format!("回写导出进度失败: {e}"))?;
    Ok(n > 0)
}

/// 一个任务的结局。终态由执行者自己写；取消（4）是取消端点写的，执行者再写一遍就会
/// 把「已取消」盖回「已完成」（见 `finish_sql` 的谓词），所以要能区分出来。
enum RunEnd {
    /// 已完成：行（状态 2、行数、文件路径/大小、进度）在 `run_one` 收尾那一写里落好，
    /// 调用方不用再补一笔
    Done,
    /// 行已不归本执行者：取消 / 被新执行者接管 / 行被删。行一个字段都不动；
    /// 成品按 `should_remove_artifact` 的判定处理（取消/删除才撤，被接管留给赢家）
    Lost,
}

/// 跑一个任务：按发起人重算数据范围 → 取数写文件（.part → 改名）。
/// 每批开始前回写进度 + 心跳（`beat`，带本代次 token），顺带看这一行还在不在自己手上：
/// 不在（取消 / 被接管）立刻收手 —— 所以取消在**批间**生效（最多多跑一批 5000 行）。
async fn run_one(state: &AppState, task: &ExportTask, seen: NaiveDateTime) -> Result<RunEnd, String> {
    let kind = spec(&task.kind).ok_or_else(|| format!("未知导出类型「{}」", task.kind))?;
    let scope = owner_scope(state, task.admin_id).await?;
    let params: JsonVal =
        serde_json::from_str(&task.params).map_err(|e| format!("筛选参数损坏: {e}"))?;

    let dir = dir(state);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建导出目录 {} 失败: {e}", dir.display()))?;
    let path = dir.join(format!("{}.csv", task.id));
    // 半份文件按**代次**隔离（`{id}.{started_at}.csv.part`）：同一行被重新认领后，新旧
    // 执行者各写各的文件，不会互相截断；输家的 `PartFile` 也只会删自己那份
    let part = part_path(&dir, task.id, seen);
    // 半份文件不留：写盘/取名任何一步失败（或 panic）都清掉 .part（口径同 backup.rs）。
    // 成功时文件已改名搬走，这里的 remove 只会拿到 NotFound —— 不用单独解除。
    let _part_guard = PartFile(part.clone());
    // 0600：导出文件里是日志/登录信息，同机其他用户不该读得到
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&part)
        .map_err(|e| format!("建导出文件失败: {e}"))?;

    let mut buf = String::from(csv::BOM);
    // BOM + 表头先落盘：一行数据都没有也是一份合法（只有表头）的 CSV
    buf.push_str(&csv::row(&(kind.headers)()));
    file.write_all(buf.as_bytes()).map_err(|e| format!("写导出文件失败: {e}"))?;
    let mut rows = 0i64;
    let mut last = None;
    loop {
        // 取数之前先报进度 + 心跳 + 看归属：取消最多等一批生效，心跳保证长任务不被误判
        if !beat(state, task.id, seen, rows).await? {
            return Ok(RunEnd::Lost);
        }
        let batch = (kind.batch)(state.clone(), params.clone(), scope.clone(), last)
            .await
            .map_err(|e| match e {
                ApiError::BadRequest(m) => m,
                other => format!("{other:?}"),
            })?;
        if batch.is_empty() {
            break;
        }
        let full = batch.len() == csv::BATCH;
        let mut buf = String::new();
        for (id, line) in batch {
            last = Some(id);
            rows += 1;
            buf.push_str(&line);
        }
        // 攒够一批就落盘：文件多大都不占内存（与流式导出同一个口径）
        file.write_all(buf.as_bytes()).map_err(|e| format!("写导出文件失败: {e}"))?;
        if !full {
            break;
        }
    }
    file.sync_all().map_err(|e| format!("落盘失败: {e}"))?;
    drop(file);
    // 改名与抢终态的顺序有讲究，两头都要照顾：
    // - 改名（I/O）失败要在**行还是 1** 的时候暴露，`run_claimed` 才能把它记成失败行；
    // - 抢终态用 `finish`（谓词 `AND status = 1`）：用户在最后一批写完的那一刻点了取消，
    //   行已经是 4，这一写命中 0 行 —— 那就把刚改好的正品**撤掉**（取消的语义是不留成品），
    //   半份交给看守清。于是「行是 4、盘上却有 .csv」这种孤儿不存在。
    std::fs::rename(&part, &path).map_err(|e| format!("导出文件改名失败: {e}"))?;
    let size = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
    // 路径与终态同一次写回（带本代次 token）：清理与下载都以行为准，不信目录里扫出来的
    // 文件名。没写进去（取消赢 / 被接管 / 写库失败）就交给 `settle_lost` 按行的现状决定
    // 盘上这份成品撤不撤 —— 不再无脑删（那样会把赢家的文件删掉，见 M1/I2）
    let msg = format!("导出 {rows} 行");
    if !finish(state, task.id, seen, 2, rows, size, &path.to_string_lossy(), &msg).await {
        settle_lost(state, task.id, &path).await;
        return Ok(RunEnd::Lost);
    }
    Ok(RunEnd::Done)
}

/// 半份文件的代次路径：`{id}.{started_at}.csv.part`（`%Y%m%d%H%M%S%6f`，20 位数字）。
/// 代次就是 fencing token，同一行重新认领后两位执行者各写各的，互不截断。
fn part_path(dir: &Path, id: i64, seen: NaiveDateTime) -> PathBuf {
    dir.join(format!("{id}.{}.csv.part", seen.format("%Y%m%d%H%M%S%6f")))
}

/// 终态那一写没命中之后，盘上成品撤不撤。**先读行的现状**（M1：写库失败 ≠ 取消赢）：
/// - 行没了（运行中被管理员删掉）或 status=4（取消赢）：撤 —— 取消的语义是不留成品；
/// - 其他（写库失败、行还是 1 被新执行者接管、赢家已经写了 2/3）：留着 —— 成品与赢家
///   可能共用同一个最终路径，撤了就是「status=2 却没有文件」。
/// 单独成函数是为了能被**无库单测**钉住（同 `claim_sql`）。
fn should_remove_artifact(row: Option<&ExportTask>) -> bool {
    match row {
        None => true,
        Some(t) => t.status == 4,
    }
}

/// 收尾失败后的善后：查一次行的现状，按 [`should_remove_artifact`] 决定去留。
/// 查行本身失败时**不动成品**（保守：删错了没法恢复，留着最多是下一轮清理收走）。
async fn settle_lost(state: &AppState, id: i64, path: &Path) {
    let row = match ExportTask::query().filter_eq("id", id) {
        Ok(q) => q.one(&state.db).await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match row {
        Ok(row) => {
            if should_remove_artifact(row.as_ref()) {
                let _ = std::fs::remove_file(path);
                tracing::info!("导出任务 {id} 的终态没写回（已取消或行已删），撤下成品");
            } else {
                tracing::info!("导出任务 {id} 的终态没写回（写库失败或被接管），成品留给行当前的归属");
            }
        }
        Err(e) => tracing::warn!("导出任务 {id} 终态判定查行失败，保守起见不动成品: {e}"),
    }
}

/// 未完成的 `.part` 的看守：`run_one` 活着它就活着，Drop 时清掉残留。
/// 正常路径先 rename 再返回，Drop 时文件已不在（remove 得到 NotFound，无害）。
struct PartFile(PathBuf);
impl Drop for PartFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 写终态的语句。谓词除了 `status = 1`，还带 `started_at = ?`（本代次 token）：
/// - **取消赢**：用户在任务写完最后一批的同时点了取消，行已经是 4，这一写命中 0 行，
///   不会把「已取消」盖成「已完成」；
/// - **归属 fencing**：行被判超时后由新执行者重新认领（started_at 换了），旧执行者
///   复活后的这一写也命中 0 行，不会覆盖赢家的终态，也不会去动赢家的文件。
/// 文件路径/大小与终态**同一次写**（不再单独 UPDATE），清理与下载都以行为准。
/// 同 `claim_sql` 的理由，谓词钉在单测里。
fn finish_sql() -> &'static str {
    "UPDATE export_task SET status = ?, row_count = ?, file_size = ?, file_path = ?, finished_at = ?, msg = ?, progress_rows = ? WHERE id = ? AND status = 1 AND started_at = ?"
}

/// 写终态（成功/失败都走这里）。返回「这一写有没有命中」（谓词 `status = 1` 且 token 相符）。
/// **不返回错误**：命中 0 行不是错误（取消 / 被接管 / 行被删），写库真失败也帮不上忙了 ——
/// 落日志等人工看。成功路径用返回值决定成品去留（见 `run_one` 尾与 `settle_lost`）。
async fn finish(
    state: &AppState,
    id: i64,
    seen: NaiveDateTime,
    status: i8,
    rows: i64,
    size: i64,
    path: &str,
    msg: &str,
) -> bool {
    let args = [
        Value::from(status),
        Value::from(rows),
        Value::from(size),
        Value::from(path),
        Value::from(now()),
        Value::from(msg),
        Value::from(rows),
        Value::from(id),
        Value::from(seen),
    ];
    match state.db.execute(finish_sql(), &args).await {
        Ok(0) => {
            tracing::info!("导出任务 {id} 的终态没写进去：行已不归本执行者（取消/接管/删除）");
            false
        }
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("回写导出任务 {id} 终态失败: {e}");
            false
        }
    }
}

/// 发起人当前的数据范围（超管 = 全部；其余按角色，规则与登录用户同一份）。
async fn owner_scope(state: &AppState, admin_id: i64) -> Result<DataScope, String> {
    let admin = Admin::query()
        .filter_eq("id", admin_id)
        .map_err(|e| e.to_string())?
        .one(&state.db)
        .await
        .map_err(|e| format!("查导出发起人失败: {e}"))?
        .ok_or("发起人已不存在")?;
    if admin.is_super == 1 {
        return Ok(DataScope { all: true, me: admin_id, ..Default::default() });
    }
    let ids = state
        .db
        .get_relations("admin_role", ("admin_id", admin_id), "role_id")
        .await
        .map_err(|e| format!("查发起人角色失败: {e}"))?;
    let roles: Vec<Role> = if ids.is_empty() {
        Vec::new()
    } else {
        Role::query()
            .filter_in("id", &crate::api::ids_to_values(&ids))
            .map_err(|e| e.to_string())?
            .all(&state.db)
            .await
            .map_err(|e| format!("查发起人角色失败: {e}"))?
    };
    datascope::from_roles(&state.db, admin_id, admin.dept_id, &roles)
        .await
        .map_err(|e| format!("{e:?}"))
}

/// `export_clean`：删过期任务（记录 + 文件）。
pub async fn clean(state: AppState) -> Result<String, String> {
    // 只清**有结局的**（2 完成 / 3 失败 / 4 已取消）：待处理/执行中的行就算过期也还等着被跑，
    // 删了等于让用户的任务凭空消失（文件都还没生成）—— 任务跑完再轮到清理。
    // 取消的行必须在这个名单里（衔接 1）：取消之后没有人会再碰它，漏了就永远删不掉
    let expired = ExportTask::query()
        .filter_lt("expire_at", now())
        .map_err(|e| e.to_string())?
        .filter_in("status", &[Value::from(2i8), Value::from(3i8), Value::from(4i8)])
        .map_err(|e| e.to_string())?
        .all(&state.db)
        .await
        .map_err(|e| format!("查过期导出失败: {e}"))?;

    let (mut files, mut rows) = (0u32, 0u32);
    for task in expired {
        // 行里没有路径也要按约定兜底删一次：改名成功、回写 file_path 之前进程死掉的话，
        // 文件已经以 `{id}.csv` 落在盘上了，只信行里的 path 会把孤儿永久留在磁盘上
        let path = if task.file_path.is_empty() {
            dir(&state).join(format!("{}.csv", task.id)).to_string_lossy().into_owned()
        } else {
            task.file_path.clone()
        };
        match std::fs::remove_file(&path) {
            Ok(_) => files += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                // 文件删不掉就**留着这行**：记录先删了，文件就再没人负责清了
                tracing::warn!("删导出文件 {} 失败，保留记录下次再试: {e}", path);
                continue;
            }
        }
        state
            .db
            .execute("DELETE FROM export_task WHERE id = ?", &[Value::from(task.id)])
            .await
            .map_err(|e| format!("删过期导出 {} 失败: {e}", task.id))?;
        rows += 1;
    }

    // 半份文件的收尾（批次 E T5）：`{id}.csv.part` 的两条正常归宿（改名成 `.csv`、
    // 或 `PartFile` 看守在 drop 时清掉）都兜不住「进程被 SIGKILL」「运行中的行被管理员
    // 删掉」和「任务被取消」——那样的半份从此没人认领。收不收只看**行的现状**，
    // 判据收在 `collectable()` 里（同一套口径也在单测里钉着）。
    let stale = now() - chrono::Duration::seconds(STALE_CLAIM_SECS);
    let mut parts = 0u32;
    let export_dir = dir(&state);
    for (id, path) in part_files(&export_dir).map_err(|e| format!("读导出目录失败: {e}"))? {
        let row = ExportTask::query()
            .filter_eq("id", id)
            .map_err(|e| e.to_string())?
            .one(&state.db)
            .await
            .map_err(|e| format!("查导出任务 {id} 失败: {e}"))?;
        if !collectable(row.as_ref(), now(), stale) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(_) => parts += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("删残留半份文件 {} 失败: {e}", path.display()),
        }
    }
    Ok(format!("清理过期导出 {rows} 个（删文件 {files} 个），收走残留 .part {parts} 个"))
}

/// 残留 `.part` 收不收：按**行的现状**判 —— 行没了（运行中被删）/ 行已有结局
/// （2 完成 / 3 失败 / **4 已取消**）才收；执行中的行要**过期且久无心跳**（那条
/// 「进程崩过」的行，与 `export_run` 挑重跑行是同一套 `last_seen` 口径）才收；
/// 待处理与心跳还热的活任务一律不碰 —— 碰了就是把手上的活拆了。
/// 单独成函数是为了能被**无库单测**钉住（理由同 `claim_sql`）。
fn collectable(row: Option<&ExportTask>, now_: chrono::NaiveDateTime, stale: chrono::NaiveDateTime) -> bool {
    match row {
        None => true,
        Some(t) => match t.status {
            // 0 待处理：马上要跑，它自己会 truncate 重写这份文件
            0 => false,
            // 执行中：过期 + 久无心跳
            1 => t.expire_at < now_ && t.last_seen().map_or(true, |s| s < stale),
            // 2 / 3 / 4：不会有执行者再动这份文件了（终态的分支，取消的就落这里）
            2 | 3 | 4 => true,
            _ => false, // 认不出的状态：宁可不收（交给人工），也不赌它在跑
        },
    }
}

/// 扫出目录里所有导出半份：`{id}.csv.part`（旧格式/手工兜底）与
/// `{id}.{20 位代次}.csv.part`（E2 起的执行者格式）。名字对不上的一律不看 —— 目录里
/// 还有运维手放的、别的进程写的文件，名字约定是唯一的认领凭据。目录不存在 = 从没导出过，
/// 空手而归（不是错误：清理任务是卫生工作，不该因为「还没有导出过」整轮失败）。
fn part_files(dir: &Path) -> std::io::Result<Vec<(i64, PathBuf)>> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(stem) = name.strip_suffix(".csv.part") else { continue };
        let mut segs = stem.split('.');
        let Some(id) = segs.next().and_then(|s| s.parse::<i64>().ok()) else { continue };
        // 代次段可有可无；有就必须是 part_path() 产出的 20 位数字，多一段都不认
        let ok = match segs.next() {
            None => true,
            Some(g) => g.len() == 20
                && g.bytes().all(|b| b.is_ascii_digit())
                && segs.next().is_none(),
        };
        if ok {
            out.push((id, path));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
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
}
