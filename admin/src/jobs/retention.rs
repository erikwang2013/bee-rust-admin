// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 日志保留与孤儿文件清理任务体。

use super::*;
use crate::backup::is_stale;

/// 日志保留：清 `login_log` / `audit_log` / `job_log` 里超过 `[log] retain_days` 的行。
/// 沿用 A4 的实现；`retain_days = 0`（永久保留）时只跳过日志，仍跑关系表兜底清理。
///
/// 关系表兜底（#8）搭这趟每天的车：角色/部门/菜单删除是「先查引用再删」，
/// 与并发的授权/建人之间有窗口，可能留下指向已删对象的关联行。清理按天跑，
/// 所以是**最终一致**（窗口最长一个任务周期），不是删除瞬间的强一致；
/// SQL 幂等，单实例与多实例都可安全执行。
pub(super) async fn log_retention(state: AppState) -> Result<String, String> {
    let days = state.cfg.retain_days;
    let logs = if days <= 0 {
        "retain_days=0（永久保留），跳过日志".to_string()
    } else {
        let n = crate::retention::purge_all(&state.db, days).await;
        format!("清理过期日志 {n} 条（保留 {days} 天）")
    };
    let relations = relation_orphan_clean(&state).await?;
    Ok(format!("{logs}；{relations}"))
}

/// 删悬挂关联行 + 修正指向已删部门的 `admin.dept_id`（见 [`log_retention`] 的说明）。
async fn relation_orphan_clean(state: &AppState) -> Result<String, String> {
    let deletes = [
        (
            "admin_role",
            "DELETE ar FROM admin_role ar LEFT JOIN `role` r ON r.id = ar.role_id \
             WHERE r.id IS NULL",
        ),
        (
            "role_menu",
            "DELETE rm FROM role_menu rm LEFT JOIN `role` r ON r.id = rm.role_id \
             LEFT JOIN menu m ON m.id = rm.menu_id WHERE r.id IS NULL OR m.id IS NULL",
        ),
        (
            "role_dept",
            "DELETE rd FROM role_dept rd LEFT JOIN `role` r ON r.id = rd.role_id \
             LEFT JOIN dept d ON d.id = rd.dept_id WHERE r.id IS NULL OR d.id IS NULL",
        ),
    ];
    let mut counts = Vec::with_capacity(deletes.len());
    for (table, sql) in deletes {
        let n = state
            .db
            .execute(sql, &[])
            .await
            .map_err(|e| format!("清理 {table} 悬挂关联失败: {e}"))?;
        counts.push(format!("{table} {n}"));
    }
    // 部门已被删、管理员还挂着它：回根（0），别让管理员永远落在不存在的部门里
    let admins = state
        .db
        .execute(
            "UPDATE admin a LEFT JOIN dept d ON d.id = a.dept_id \
             SET a.dept_id = 0 WHERE a.dept_id <> 0 AND d.id IS NULL",
            &[],
        )
        .await
        .map_err(|e| format!("修正悬挂部门失败: {e}"))?;
    Ok(format!("清理悬挂关联 {}，修正管理员部门 {admins}", counts.join(" / ")))
}

/// 头像孤儿清理：删头像组目录（`{upload_dir}/avatar/`）下**不再被任何 admin 引用**的
/// 头像文件。引用形态两种都认（4b 前后各一种）：
/// - 列里是插件的 `saved_path`（`avatar_202610_<md5>.png`）→ 按上传根解析成相对路径；
/// - 列里是老 URL（`/api/v1/avatar/xxx`）→ 老落盘路径是 `{id}.png` / `{id}.jpg`，
///   两种扩展名都算引用 —— 存量头像没迁移前仍在用，误删就是把别人的头像删没了。
///
/// **护栏：只删扩展名在 png/jpg 白名单里的文件，其他一律不碰。**
/// 看起来像可以删掉的啰嗦，其实是在防误删 —— 目录里除了成品头像还有：
/// 插件写到一半的临时文件（`<临时名>.png.part`，扩展名是 `part`；没有引用关系，
/// 只能按年龄收，见 [`clean_upload_temps`]）、运维/用户手放的文件（备份、说明、
/// 别人家的上传；这些没有引用关系可依赖，一律不碰）。
/// 反过来说：白名单内、又没人引用、且过了保护期（`too_fresh`）的 png/jpg 就是孤儿
/// （包括用户手放的），该删。留着的代价只是占点磁盘，删错的代价是丢数据 —— 划界就按这条。
pub(super) async fn avatar_orphan_clean(state: AppState) -> Result<String, String> {
    let dir = std::path::Path::new(&state.cfg.upload_dir).join("avatar");
    let files = match collect_files(&dir) {
        Ok(files) => files,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok("头像目录不存在，无需清理".into());
        }
        Err(e) => return Err(format!("读头像目录 {} 失败: {e}", dir.display())),
    };

    // 引用集合：相对头像组目录的路径（与下面 walk 出来的相对路径同一坐标系）
    let admins = Admin::query().all(&state.db).await.map_err(|e| format!("查管理员失败: {e}"))?;
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

    let (mut seen, mut removed, mut fresh) = (0u64, 0u64, 0u64);
    for path in files {
        seen += 1;
        if !is_avatar_file(&path) {
            continue;
        }
        let Ok(rel) = path.strip_prefix(&dir) else { continue };
        if kept.contains(rel) {
            continue;
        }
        // 与附件同一个保护期：上传是「先落盘、后落库」，撞进这个窗口就会删掉马上要用的头像
        if too_fresh(&path) {
            fresh += 1;
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
    let temps = clean_upload_temps(&state.cfg.upload_dir, crate::api::avatar::AVATAR_GROUP);
    Ok(format!(
        "头像 {seen} 个，删除孤儿 {removed} 个，保护期内跳过 {fresh} 个，清理临时文件 {temps} 个"
    ))
}

/// 附件孤儿清理：删附件组目录（`{upload_dir}/attach/`）下**不再被任何 `attachment` 行
/// 引用**的文件。护栏与头像同一条：只删白名单扩展名里的文件 —— 目录里除了成品还有
/// 插件写到一半的临时文件（`<临时名>.png.part`；按年龄单独收，见 [`clean_upload_temps`]）
/// 与运维手放的东西，那些不碰。
///
/// 与头像的差别：附件的引用集合只有一个来源（`attachment.stored_path`，没有历史 URL 形态），
/// 所以这里简单得多。上传成品落盘成功、INSERT 失败的那条缝（以及被人手删的行）只有它兜得住。
pub(super) async fn attach_orphan_clean(state: AppState) -> Result<String, String> {
    let dir = std::path::Path::new(&state.cfg.upload_dir).join(crate::api::attach::ATTACH_GROUP);
    let files = match collect_files(&dir) {
        Ok(files) => files,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok("附件目录不存在，无需清理".into());
        }
        Err(e) => return Err(format!("读附件目录 {} 失败: {e}", dir.display())),
    };

    let rows = Attachment::query().all(&state.db).await.map_err(|e| format!("查附件失败: {e}"))?;
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
    // 每小时顺带收一次临时文件（头像任务每天一趟）：两个任务都扫 `_header` 无害
    let temps = clean_upload_temps(&state.cfg.upload_dir, crate::api::attach::ATTACH_GROUP);
    Ok(format!(
        "附件 {seen} 个，删除孤儿 {removed} 个，保护期内跳过 {fresh} 个，清理临时文件 {temps} 个"
    ))
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
                tracing::warn!("读上传子目录 {} 失败: {e}", dir.display());
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

/// 回收上传遗留的临时文件：`_header/` 断点文件（插件独占的目录，整目录都是它的）
/// 与分组目录下的 `*.part` 半成品 —— 两种都不在成品白名单里，孤儿清理不会碰。
/// 头像/附件两个任务都调（`_header` 重复扫无害），残留最多多留一轮。
fn clean_upload_temps(upload_dir: &str, group: &str) -> u64 {
    let root = std::path::Path::new(upload_dir);
    clean_temp_files(&root.join("_header"), |_| true)
        + clean_temp_files(&root.join(group), is_part_file)
}

/// 分组目录里的半成品命名：`<临时名>.<扩展名>.part`（插件 `PartialResource`）。
fn is_part_file(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("part")
}

/// 删 `dir` 下（递归）超过年龄线（[`crate::backup::TEMP_MAX_AGE_SECS`]）的临时文件；
/// 目录不存在不算错，读/删失败告警后继续 —— 卫生任务不因一个坏目录整轮翻车
/// （与孤儿清理同一条护栏）。
fn clean_temp_files(dir: &std::path::Path, is_temp: fn(&std::path::Path) -> bool) -> u64 {
    let files = match collect_files(dir) {
        Ok(files) => files,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => {
            tracing::warn!("读临时目录 {} 失败: {e}", dir.display());
            return 0;
        }
    };
    let mut removed = 0;
    for path in files {
        if !is_temp(&path) || !is_stale(&path) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(_) => removed += 1,
            Err(e) => tracing::warn!("删临时文件 {} 失败: {e}", path.display()),
        }
    }
    removed
}

/// 删掉成品文件后，把它的秒传索引一起清掉。
///
/// 索引里可能还留着 `group_hash → saved_path` 指向这个已删文件（管理员行被删、值被直接
/// 改库 —— 文件成了孤儿但索引没过期），不清的话下次有人传同样的内容会「秒传命中」到一个
/// 死链（头像 404）。`rel` 是相对**该分组目录**的路径，只有「一层子目录 + 文件名」这种
/// 插件成品的位置才构造得出 saved_path；直接躺在组目录下的老文件/手工文件跳过。
fn purge_instant(state: &AppState, group: &str, rel: &std::path::Path) {
    let (Some(subdir), Some(name)) =
        (rel.parent().and_then(|p| p.to_str()), rel.file_name().and_then(|n| n.to_str()))
    else {
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
/// 成品头像；`.part` 交给 [`clean_upload_temps`] 按年龄收，`.bak`、无扩展名的一律
/// 留给人工处理。
fn is_avatar_file(path: &std::path::Path) -> bool {
    in_ext_whitelist(path, &crate::api::avatar::AVATAR_EXTS)
}

/// 附件同理：只有 `ATTACH_EXTS` 里的扩展名才可能是上传成品。
fn is_attach_file(path: &std::path::Path) -> bool {
    in_ext_whitelist(path, &crate::api::attach::ATTACH_EXTS)
}

/// 孤儿文件的保护期（批次 E S2），头像与附件共用。上传是「先落盘、后 INSERT」，两者之间
/// 有毫秒级窗口；附件删除也改成「只删行、文件交给清理」（延迟回收）—— 有这条下限，刚写下
/// 的文件不会被任何一轮清理误收。5 分钟：比任何一次上传耗时都长得多，又短到磁盘不会被垃圾拖住。
pub(super) const ATTACH_MIN_AGE_SECS: u64 = 300;

/// 文件「太新」= 保护期内，清理不碰。读不到 mtime（个别文件系统 / 文件刚被并发删掉）
/// 一律按太新处理：宁可晚一轮收（最多多留一天），不可删错。
pub(super) fn too_fresh(path: &std::path::Path) -> bool {
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
    use super::{clean_temp_files, clean_upload_temps, is_part_file};
    use crate::backup::{TEMP_MAX_AGE_SECS, is_stale};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 每个测试独占一个临时根（pid + 自增序号）：同进程并行跑测试也不会互相踩。
    fn temp_root(tag: &str) -> PathBuf {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bee_job_temps_{}_{tag}_{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"x").unwrap();
    }

    /// 把 mtime 拨到 24 小时年龄线之外 —— 不用 sleep，测试保持确定且快。
    fn age(path: &Path) {
        let old =
            std::time::SystemTime::now() - std::time::Duration::from_secs(TEMP_MAX_AGE_SECS + 60);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(old).unwrap();
    }

    /// 上传临时文件按年龄收：`_header` 整目录收，分组目录只收 `.part`；
    /// 24 小时内的、非 `.part` 的手工文件、另一分组的东西都不碰。
    #[test]
    fn clean_upload_temps_collects_stale_header_and_part_only() {
        let root = temp_root("upload");
        let (header, avatar, attach) =
            (root.join("_header"), root.join("avatar"), root.join("attach"));
        for dir in [&header, &avatar, &attach] {
            std::fs::create_dir_all(dir).unwrap();
        }
        // _header 是插件独占目录：整目录按年龄收，不看扩展名
        touch(&header.join("fresh.hdr"));
        touch(&header.join("stale.hdr"));
        age(&header.join("stale.hdr"));
        // avatar：只收 `.part`；`.txt` / `.bak` 是手工文件，即使超龄也不碰
        touch(&avatar.join("fresh.png.part"));
        touch(&avatar.join("stale.png.part"));
        touch(&avatar.join("notes.txt"));
        touch(&avatar.join("old.bak"));
        for name in ["stale.png.part", "notes.txt", "old.bak"] {
            age(&avatar.join(name));
        }
        // attach：同一条 `.part` 规则，留给附件那一趟
        touch(&attach.join("fresh.png.part"));
        touch(&attach.join("stale.png.part"));
        age(&attach.join("stale.png.part"));

        let root = root.to_str().unwrap();
        // 头像任务：收 `_header` 的超龄断点 + avatar 的超龄半成品，不扫 attach
        assert_eq!(clean_upload_temps(root, "avatar"), 2);
        assert!(!header.join("stale.hdr").exists(), "超龄断点要删");
        assert!(header.join("fresh.hdr").exists(), "24 小时内的断点不能删");
        assert!(!avatar.join("stale.png.part").exists(), "超龄半成品要删");
        assert!(avatar.join("fresh.png.part").exists(), "进行中的上传不能删");
        assert!(avatar.join("notes.txt").exists(), "非 .part 的手工文件不碰");
        assert!(avatar.join("old.bak").exists(), "非 .part 的手工文件不碰");
        assert!(attach.join("stale.png.part").exists(), "另一分组这一趟不扫");

        // 附件任务：收 attach 的超龄半成品（_header 已空）
        assert_eq!(clean_upload_temps(root, "attach"), 1);
        assert!(!attach.join("stale.png.part").exists());
        assert!(attach.join("fresh.png.part").exists(), "进行中的上传不能删");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// `clean_temp_files` 的缺目录 / 新文件 / 读不到 mtime 三条护栏：
    /// 任何一条失守都会删掉上游正在用的文件。
    #[test]
    fn clean_temp_files_keeps_fresh_and_unreadable_entries() {
        let root = temp_root("single");
        let dir = root.join("avatar");
        std::fs::create_dir_all(&dir).unwrap();
        touch(&dir.join("fresh.png.part")); // 刚写：留
        touch(&dir.join("stale.png.part")); // 超龄：删
        touch(&dir.join("stale.txt")); // 谓词不匹配：留
        age(&dir.join("stale.png.part"));
        age(&dir.join("stale.txt"));

        assert_eq!(clean_temp_files(&dir, is_part_file), 1);
        assert!(dir.join("fresh.png.part").exists(), "新文件在保护期内");
        assert!(!dir.join("stale.png.part").exists());
        assert!(dir.join("stale.txt").exists(), "不是临时文件命名，不碰");

        // 目录不存在不是错：两趟任务都会扫 `_header`，缺目录必须安静返回 0
        assert_eq!(clean_temp_files(&root.join("_header"), |_| true), 0);
        assert_eq!(clean_upload_temps(root.to_str().unwrap(), "attach"), 0, "分组目录缺失也不报错");

        // 读不到 mtime（文件已被并发删掉）一律当「新」：宁可晚一轮收，不可删错
        assert!(!is_stale(&dir.join("gone.png.part")));

        let _ = std::fs::remove_dir_all(&root);
    }
}
