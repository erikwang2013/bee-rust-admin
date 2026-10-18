// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! `export_run` 执行器：认领 / 心跳 / 取数 / 终态。

use super::*;

/// 进程内同时跑几个导出任务。取数批接口是 `LIMIT 5000` 的顺序读，开 2 条既能把
/// 「一个慢任务堵住别人」摊平，又不至于把库压垮（后台任务，慢一点没关系）。
/// 上限是**进程级**的：`gate()` 里那一个信号量由所有 `dispatch()` 调用共享 —— 跨 tick /
/// 多次手动触发也不会叠加（旧实现每次派发新建一个，两批各跑各的，实际能超过 2）。
/// 同一行只被一个执行者认领由 `claim` 的排他谓词保证，与本闸是两回事。
const EXPORT_CONCURRENCY: usize = 2;
const _: () = assert!(EXPORT_CONCURRENCY > 0, "并发闸给 0 会死等");

/// 进程级并发闸（与 job Busy 的进程内语义一致）：懒建一次，之后所有派发共用。
/// 只在本模块里取用，没人会 `close()` 它 —— `acquire_owned` 因此不会返回错误。
pub(super) fn gate() -> Arc<tokio::sync::Semaphore> {
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
pub(super) fn dispatch<F, Fut, T>(sem: Arc<tokio::sync::Semaphore>, jobs: Vec<F>)
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
pub(super) fn claim_sql(status: i8) -> &'static str {
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
pub(super) fn beat_sql() -> &'static str {
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
    // 成品与半份都按**代次**命名（`{id}.{started_at}.csv[.part]`，N1）：同一行被重新
    // 认领后，新旧执行者各写各的文件，既不互相截断，输家的改名也不会覆盖赢家的成品
    let path = final_path(&dir, task.id, seen);
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

/// 成品的代次路径：`{id}.{started_at}.csv`（`%Y%m%d%H%M%S%6f`，20 位数字）。
/// 老执行者即使跑完改名，也只会写自己那一代的名字，覆盖不了赢家的成品（N1）。
pub(super) fn final_path(dir: &Path, id: i64, seen: NaiveDateTime) -> PathBuf {
    dir.join(format!("{id}.{}.csv", seen.format("%Y%m%d%H%M%S%6f")))
}

/// 半份文件的代次路径：`{id}.{started_at}.csv.part`（`%Y%m%d%H%M%S%6f`，20 位数字）。
/// 代次就是 fencing token，同一行重新认领后两位执行者各写各的，互不截断。
pub(super) fn part_path(dir: &Path, id: i64, seen: NaiveDateTime) -> PathBuf {
    dir.join(format!("{id}.{}.csv.part", seen.format("%Y%m%d%H%M%S%6f")))
}

/// 终态那一写没命中之后，盘上成品撤不撤。**先读行的现状**（M1：写库失败 ≠ 取消赢）：
/// - 行没了（运行中被管理员删掉）或 status=4（取消赢）：撤 —— 取消的语义是不留成品；
/// - 其他（写库失败、被新执行者接管、赢家已经写了 2/3）：留着 —— 这一份是本代次的
///   成品名，删不删都不影响赢家；留着的由 `export_clean` 按名字约定收走。
/// 单独成函数是为了能被**无库单测**钉住（同 `claim_sql`）。
pub(super) fn should_remove_artifact(row: Option<&ExportTask>) -> bool {
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
pub(super) fn finish_sql() -> &'static str {
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
