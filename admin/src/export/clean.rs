// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! `export_clean` 清理任务与残留 `.part` 回收。

use super::*;

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
    let export_dir = dir(&state);
    for task in expired {
        // 成品按代次命名（N1）：行里的 `file_path` 指向赢家那份，而输家（被接管）与
        // 升级前留下的旧代次成品都按名字约定扫。行里没有路径也要扫 —— 改名成功、
        // 回写 file_path 之前进程死掉的话，只信行里的 path 会把孤儿永久留在磁盘上（N5）。
        // 清单与接口「删任务连文件」共用 [`artifact_paths`]，两处不各写一遍。
        let paths = artifact_paths(&export_dir, &task).map_err(|e| format!("读导出目录失败: {e}"))?;
        let mut failed = false;
        for path in &paths {
            match std::fs::remove_file(path) {
                Ok(_) => files += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    // 文件删不掉就**留着这行**：记录先删了，文件就再没人负责清了
                    tracing::warn!("删导出文件 {} 失败，保留记录下次再试: {e}", path.display());
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            continue;
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
pub(super) fn collectable(row: Option<&ExportTask>, now_: chrono::NaiveDateTime, stale: chrono::NaiveDateTime) -> bool {
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

/// 扫出某个任务的成品：旧命名 `{id}.csv`（升级前的成品）与代次命名
/// `{id}.{20 位代次}.csv`（N1 起的执行者格式）。名字对不上的一律不看 ——
/// `{id}.csv.part` 是半份，另有回收路径。
pub(super) fn final_files(dir: &Path, id: i64) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    let old = format!("{id}.csv");
    let prefix = format!("{id}.");
    for entry in entries {
        // 目录项读取失败要向上报：把「读不到」当成「没有文件」会让调用方删掉行，
        // 而那份没扫到的文件从此再没人负责清。
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let ok = name == old
            || name.strip_prefix(&prefix).and_then(|s| s.strip_suffix(".csv")).is_some_and(|g| {
                g.len() == 20 && g.bytes().all(|b| b.is_ascii_digit())
            });
        if ok {
            out.push(path);
        }
    }
    Ok(out)
}

/// 扫出目录里所有导出半份：`{id}.csv.part`（旧格式/手工兜底）与
/// `{id}.{20 位代次}.csv.part`（E2 起的执行者格式）。名字对不上的一律不看 —— 目录里
/// 还有运维手放的、别的进程写的文件，名字约定是唯一的认领凭据。目录不存在 = 从没导出过，
/// 空手而归（不是错误：清理任务是卫生工作，不该因为「还没有导出过」整轮失败）。
pub(super) fn part_files(dir: &Path) -> std::io::Result<Vec<(i64, PathBuf)>> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for entry in entries {
        // 同 `final_files`：读不到的目录项不能悄悄跳过
        let path = entry?.path();
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
