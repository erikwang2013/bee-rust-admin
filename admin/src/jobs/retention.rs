// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 日志保留与孤儿文件清理任务体。

use super::*;

/// 日志保留：清 `login_log` / `audit_log` / `job_log` 里超过 `[log] retain_days` 的行。
/// 沿用 A4 的实现；`retain_days = 0`（永久保留）时什么都不做。
pub(super) async fn log_retention(state: AppState) -> Result<String, String> {
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
pub(super) async fn attach_orphan_clean(state: AppState) -> Result<String, String> {
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
