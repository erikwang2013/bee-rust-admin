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
//! - 文件落在 `{upload_dir}/exports/<任务 id>.csv`（先写 `.part` 再改名，中断不留半份）。
use crate::api::{audit_log, csv, login_log};
use crate::datascope::{self, DataScope};
use crate::error::ApiError;
use crate::models::{Admin, ExportTask, Role};
use crate::state::AppState;
use crate::util::now;
use crate::relations::RelationsExt;
use bee_orm::Value;
use serde_json::Value as JsonVal;
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;

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

/// 认领超时：执行中的行超过这么久还停在 1，当成「进程崩过」重跑一次。
/// shortcut: 固定 10 分钟，真要按任务类型分档再进配置（导出都是分钟级，够用）
const STALE_CLAIM_SECS: i64 = 600;

/// `export_run`：把待处理（或卡住的）任务串行跑掉。返回进 `job_log.msg` 的摘要。
pub async fn run(state: AppState) -> Result<String, String> {
    let stale = now() - chrono::Duration::seconds(STALE_CLAIM_SECS);
    let pending = ExportTask::query()
        .filter_eq("status", 0)
        .map_err(|e| e.to_string())?
        .all(&state.db)
        .await
        .map_err(|e| format!("查待处理导出失败: {e}"))?;
    let stuck = ExportTask::query()
        .filter_eq("status", 1)
        .map_err(|e| e.to_string())?
        .filter_lt("started_at", stale)
        .map_err(|e| e.to_string())?
        .all(&state.db)
        .await
        .map_err(|e| format!("查执行中的导出失败: {e}"))?;

    let (mut ok, mut failed) = (0u32, 0u32);
    for task in pending.into_iter().chain(stuck) {
        if !claim(&state, &task).await? {
            continue;
        }
        match run_one(&state, &task).await {
            Ok((rows, bytes)) => {
                finish(&state, task.id, 2, rows, bytes, &format!("导出 {rows} 行")).await?;
                ok += 1;
            }
            Err(e) => {
                tracing::warn!("导出任务 {} 失败: {e}", task.id);
                // 失败行也把 msg 落库：界面上要看得见原因，不然只能翻日志
                let msg: String = e.chars().take(255).collect();
                finish(&state, task.id, 3, 0, 0, &msg).await?;
                failed += 1;
            }
        }
    }
    Ok(format!("导出任务 {} 个（成功 {ok} / 失败 {failed}）", ok + failed))
}

/// 认领一条任务：把行推进「执行中」的那一次 UPDATE 算认领成功，别的执行者拿到 0 行。
/// 谓词必须**排他**：曾经写成 `status IN (0, 1)` 是错的 —— 第一次 UPDATE 把行改成 1 之后，
/// 第二个执行者用同样的条件照样命中（1 ∈ (0,1)），同一任务就有两个执行者（双写 .part、
/// 双 finish 互相覆盖）。所以按进入时的状态分两条：
/// - 待处理（status=0）：`status = 0` 只在第一次命中（认领后行变成 1）；
/// - 执行中超时（status=1 且 `started_at` 早于认领超时，见上面查 `stuck` 的口径）：
///   认领会把 `started_at` 刷成现在，第二个执行者的 `started_at < 超时线` 不再命中。
async fn claim(state: &AppState, task: &ExportTask) -> Result<bool, String> {
    let sql = claim_sql(task.status);
    let mut args = vec![Value::from(now()), Value::from(task.id)];
    if task.status != 0 {
        args.push(Value::from(now() - chrono::Duration::seconds(STALE_CLAIM_SECS)));
    }
    let n = state
        .db
        .execute(sql, &args)
        .await
        .map_err(|e| format!("认领导出任务 {} 失败: {e}", task.id))?;
    Ok(n > 0)
}

/// 认领用的两条语句。单独成函数是为了能被**无库单测**钉住谓词：集成测试在另一个 crate
/// 里只能复制 SQL（bin crate 不能 use 自己的代码），复制品对 src 不可证伪（老谓词回退时
/// 它照样绿），所以谓词本身的排他性在这里钉。
fn claim_sql(status: i8) -> &'static str {
    if status == 0 {
        "UPDATE export_task SET status = 1, started_at = ? WHERE id = ? AND status = 0"
    } else {
        "UPDATE export_task SET status = 1, started_at = ? WHERE id = ? AND status = 1 AND started_at < ?"
    }
}

/// 跑一个任务：按发起人重算数据范围 → 取数写文件（.part → 改名）。
async fn run_one(state: &AppState, task: &ExportTask) -> Result<(i64, i64), String> {
    let kind = spec(&task.kind).ok_or_else(|| format!("未知导出类型「{}」", task.kind))?;
    let scope = owner_scope(state, task.admin_id).await?;
    let params: JsonVal =
        serde_json::from_str(&task.params).map_err(|e| format!("筛选参数损坏: {e}"))?;

    let dir = dir(state);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建导出目录 {} 失败: {e}", dir.display()))?;
    let path = dir.join(format!("{}.csv", task.id));
    let part = dir.join(format!("{}.csv.part", task.id));
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
    std::fs::rename(&part, &path).map_err(|e| format!("导出文件改名失败: {e}"))?;

    let size = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
    // 路径记进任务行：清理与下载都以行为准，不信目录里扫出来的文件名
    state
        .db
        .execute(
            "UPDATE export_task SET file_path = ?, file_size = ? WHERE id = ?",
            &[Value::from(path.to_string_lossy().as_ref()), Value::from(size), Value::from(task.id)],
        )
        .await
        .map_err(|e| format!("回写导出文件失败: {e}"))?;
    Ok((rows, size))
}

/// 未完成的 `.part` 的看守：`run_one` 活着它就活着，Drop 时清掉残留。
/// 正常路径先 rename 再返回，Drop 时文件已不在（remove 得到 NotFound，无害）。
struct PartFile(PathBuf);
impl Drop for PartFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 写终态（成功/失败都走这里）。
async fn finish(
    state: &AppState,
    id: i64,
    status: i8,
    rows: i64,
    size: i64,
    msg: &str,
) -> Result<(), String> {
    state
        .db
        .execute(
            "UPDATE export_task SET status = ?, row_count = ?, file_size = ?, finished_at = ?, msg = ? WHERE id = ?",
            &[
                Value::from(status),
                Value::from(rows),
                Value::from(size),
                Value::from(now()),
                Value::from(msg),
                Value::from(id),
            ],
        )
        .await
        .map_err(|e| format!("回写导出任务 {id} 失败: {e}"))?;
    Ok(())
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
    // 只清**跑完的**（2 完成 / 3 失败）：待处理/执行中的行就算过期也还等着被跑，
    // 删了等于让用户的任务凭空消失（文件都还没生成）—— 任务跑完再轮到清理
    let expired = ExportTask::query()
        .filter_lt("expire_at", now())
        .map_err(|e| e.to_string())?
        .filter_in("status", &[Value::from(2i8), Value::from(3i8)])
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
    // 或 `PartFile` 看守在 drop 时清掉）都兜不住「进程被 SIGKILL」和「运行中的行被
    // 管理员删掉」——那样的半份从此没人认领。按**行的现状**决定收不收：
    // 行没了 / 行已终态 / 行卡在执行中且过期又超时（与 `export_run` 的认领超时同一口径）
    // 才收；待处理与正在跑的一律不碰 —— 碰了就是把手上的活拆了。
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
        let collectable = match &row {
            None => true, // 行已经没了（运行中被删）
            Some(t) if t.status == 2 || t.status == 3 => true, // 终态还留着半份
            // 执行中：过期又超时 = 那条「进程崩过」的行
            Some(t) if t.status == 1 => t.expire_at < now() && t.started_at.map_or(true, |s| s < stale),
            Some(_) => false, // 0 待处理：马上要跑，它自己会 truncate 重写这份文件
        };
        if !collectable {
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

/// 扫出目录里所有 `{id}.csv.part`。名字对不上的一律不看 —— 目录里还有运维手放的、
/// 别的进程写的文件，名字约定是唯一的认领凭据。目录不存在 = 从没导出过，空手而归
/// （不是错误：清理任务是卫生工作，不该因为「还没有导出过」整轮失败）。
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
        let Ok(id) = stem.parse::<i64>() else { continue };
        out.push((id, path));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 名字约定：`{id}.csv.part` 才是回收对象，别的名字（运维手放的、别的进程写的、
    /// id 不是数字的）一律不看；目录不存在 = 空手而归而不是报错。
    #[test]
    fn part_files_only_matches_the_naming_convention() {
        let dir = std::env::temp_dir().join(format!("bee-parts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["7.csv.part", "abc.csv.part", "readme.txt", "9.csv"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let mut got: Vec<i64> = part_files(&dir).unwrap().into_iter().map(|(id, _)| id).collect();
        got.sort_unstable();
        assert_eq!(got, vec![7], "只认 {{id}}.csv.part");
        assert!(part_files(&dir.join("nope")).unwrap().is_empty(), "目录不存在 = 空手而归");
        let _ = std::fs::remove_dir_all(&dir);
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

    /// 发起时校验 params 用的结构与取数用的一致：类型不对当场拒。
    #[test]
    fn check_rejects_wrong_param_types() {
        let k = spec("login-log").unwrap();
        assert!((k.check)(&serde_json::json!({})).is_ok(), "空筛选 = 全量，合法");
        assert!((k.check)(&serde_json::json!({"username": "a"})).is_ok());
        assert!((k.check)(&serde_json::json!({"status": "不是整数"})).is_err());
    }
}
