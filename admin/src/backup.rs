// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 数据库备份（扩展工程项 9）：内置任务 `db_backup` 的实现。
//!
//! 流程：解析 `[db] dsn` → 跑 `mysqldump` → 落盘 `{dir}/backup_<时间戳>.sql` →
//! 按 `[backup] keep` 删最旧的几份。手动触发走既有的任务执行接口（`job_log` 留痕）。
//!
//! 三个刻意的取舍：
//! - 口令走 `MYSQL_PWD` 环境变量，不进命令行（`ps` 里看不到，子进程也不落 argv）；
//! - 先写 `.part` 再改名：中断的 dump 不会以「备份文件」的身份留在目录里被当成好备份；
//! - 备份文件建的时候就 0600：dump 里是全库数据（含口令哈希），不给同机其他用户读。
//!
//! 恢复是手工动作（停服 → `mysql < dump`），步骤见 docs/usage.md「备份与恢复」。
use crate::state::AppState;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// `[db] dsn` 解析结果（mysqldump 需要的五项）。
#[derive(Debug, PartialEq)]
pub struct Dsn {
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: u16,
    pub db: String,
}

/// 解析 `mysql://user:pass@host[:port]/db[?params]`。
///
/// 凭据按 install.sh `urlenc` 的约定做百分号解码（口令里的 `% @ : / # ? & +`
/// 在 conf 里是编码形态；这是 install.sh 与 DSN 解析之间的契约）。
pub fn parse_dsn(dsn: &str) -> Result<Dsn, String> {
    let rest = dsn
        .trim()
        .strip_prefix("mysql://")
        .ok_or("DSN 必须以 mysql:// 开头")?;
    // 参数段（?ssl-mode=…）不参与解析
    let rest = rest.split('?').next().unwrap_or(rest);
    let (userinfo, host_db) = rest
        .rsplit_once('@')
        .ok_or("DSN 缺少凭据（应为 user:pass@host/db）")?;
    let (host_port, db) = host_db.split_once('/').ok_or("DSN 缺少库名")?;
    if db.is_empty() {
        return Err("DSN 库名为空".into());
    }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (
            h,
            p.parse::<u16>().map_err(|_| format!("DSN 端口非法: {p:?}"))?,
        ),
        None => (host_port, 3306),
    };
    if host.is_empty() {
        return Err("DSN 主机为空".into());
    }
    // 没有 ':' = 空口令（MySQL 允许），不是错误
    let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
    Ok(Dsn {
        user: decode(user)?,
        password: decode(password)?,
        host: host.to_string(),
        port,
        db: db.to_string(),
    })
}

/// 百分号解码。用 percent-encoding 而不是 form_urlencoded：后者把裸 `+` 解成空格，
/// 口令里真有个 `+`（手写 conf）就静默解错了。
fn decode(s: &str) -> Result<String, String> {
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.into_owned())
        .map_err(|_| format!("DSN 凭据不是合法 UTF-8（% 编码坏了？）: {s:?}"))
}

/// 备份目录：`[backup] dir` 非空用它，否则 `{upload_dir}/backups`。
pub fn backup_dir(upload_dir: &str, dir: &str) -> PathBuf {
    let dir = dir.trim();
    if dir.is_empty() {
        Path::new(upload_dir).join("backups")
    } else {
        PathBuf::from(dir)
    }
}

/// 任务体：dump → 落盘 → 清理旧份。返回写进 `job_log.msg` 的摘要。
/// 收 `AppState`（与 `jobs::JOBS` 里其他任务体同签名；内部都是 Arc，克隆廉价）。
pub async fn run(state: AppState) -> Result<String, String> {
    let cfg = &state.cfg;
    let dsn = parse_dsn(&cfg.db_dsn)?;
    let dir = backup_dir(&cfg.upload_dir, &cfg.backup_dir);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("建备份目录 {} 失败: {e}", dir.display()))?;

    let name = format!("backup_{}.sql", chrono::Local::now().format("%Y%m%d_%H%M%S"));
    let final_path = dir.join(&name);
    let tmp = dir.join(format!("{name}.part"));
    // 先建文件（0600）再交给 mysqldump：`--result-file` 打开已存在的文件不改权限，
    // 预建才能把 0600 保住（否则按 umask 通常是 0644，同机谁都能读全库 dump）。
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("创建 {} 失败: {e}", tmp.display()))?;

    let out = tokio::process::Command::new("mysqldump")
        .arg(format!("--host={}", dsn.host))
        .arg(format!("--port={}", dsn.port))
        .arg(format!("--user={}", dsn.user))
        // 一致性快照（InnoDB），不锁表；--no-tablespaces：没有 PROCESS 权限的账号
        // 默认会在查 tablespace 时被拒（MySQL 8），而恢复用不上这点信息
        .arg("--single-transaction")
        .arg("--no-tablespaces")
        .arg(format!("--result-file={}", tmp.display()))
        .arg(&dsn.db)
        .env("MYSQL_PWD", &dsn.password)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("执行 mysqldump 失败（mysql 客户端装了吗）: {e}"))?;

    if !out.status.success() {
        let _ = tokio::fs::remove_file(&tmp).await;
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "mysqldump 退出码 {:?}: {}",
            out.status.code(),
            tail(&stderr)
        ));
    }
    // 空 dump 比没有更坏（会被当成可恢复的备份），当失败处理
    let size = std::fs::metadata(&tmp)
        .map_err(|e| format!("读 {} 元数据失败: {e}", tmp.display()))?
        .len();
    if size == 0 {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err("mysqldump 产出空文件，按失败处理".into());
    }
    if let Err(e) = std::fs::rename(&tmp, &final_path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("改名 {} 失败: {e}", final_path.display()));
    }
    let removed = prune(&dir, cfg.backup_keep.max(0) as usize)
        .map_err(|e| format!("清理旧备份失败（本次备份已成功）: {e}"))?;
    Ok(format!("备份 {name}（{size} 字节）；清理旧备份 {removed} 份"))
}

/// 删最旧的备份，保最新 `keep` 份。文件名里时间戳（`YYYYmmdd_HHMMSS`）在前，
/// 字典序就是时间序；只认 `backup_*.sql`，`.part` 与用户手放的文件不碰。
fn prune(dir: &Path, keep: usize) -> Result<usize, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("读目录 {} 失败: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with("backup_") && n.ends_with(".sql")
                })
        })
        .collect();
    files.sort();
    if files.len() <= keep {
        return Ok(0);
    }
    let mut removed = 0;
    for path in &files[..files.len() - keep] {
        match std::fs::remove_file(path) {
            Ok(_) => removed += 1,
            // 删不掉只告警：本轮备份已经成功，不能因为清理翻车
            Err(e) => tracing::warn!("删旧备份 {} 失败: {e}", path.display()),
        }
    }
    Ok(removed)
}

/// stderr 的尾巴：整段进 msg 会被截到 255 字符（job_log 列宽），留结尾更有用。
fn tail(s: &str) -> String {
    let t = s.trim();
    let chars: Vec<char> = t.chars().collect();
    chars[chars.len().saturating_sub(200)..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsn_parses_host_port_db_and_decodes_credentials() {
        let d = parse_dsn("mysql://root:p%40ss@127.0.0.1:3306/bee_admin_test").unwrap();
        assert_eq!(d.user, "root");
        assert_eq!(d.password, "p@ss");
        assert_eq!(d.host, "127.0.0.1");
        assert_eq!(d.port, 3306);
        assert_eq!(d.db, "bee_admin_test");
    }

    #[test]
    fn dsn_defaults_port_and_strips_query() {
        let d = parse_dsn("mysql://u:p@db.internal/bee_admin?ssl-mode=DISABLED").unwrap();
        assert_eq!(d.host, "db.internal");
        assert_eq!(d.port, 3306, "没写端口 = 3306");
        assert_eq!(d.db, "bee_admin");
    }

    /// install.sh `urlenc` 的字符集必须全解对 —— 尤其是 `+`：
    /// 表单式解码会把裸 `+` 变成空格，那样口令就错了（用 percent_decode 而非 form_urlencoded）。
    #[test]
    fn percent_decoding_covers_install_sh_charset() {
        let d = parse_dsn("mysql://u%3A1:p%2B%2F%23%3F%26%25%40@h/db").unwrap();
        assert_eq!(d.user, "u:1");
        assert_eq!(d.password, "p+/#?&%@");
        // 裸 + 是字面量 +（install.sh 会把真 + 编成 %2B，但手写 conf 的裸 + 不该变空格）
        assert_eq!(parse_dsn("mysql://u:a+b@h/db").unwrap().password, "a+b");
        // 空口令不是错（MySQL 允许）
        assert_eq!(parse_dsn("mysql://u@h/db").unwrap().password, "");
    }

    #[test]
    fn dsn_rejects_malformed() {
        for bad in [
            "",
            "postgres://u:p@h/db",
            "mysql://127.0.0.1:3306/db",  // 没凭据
            "mysql://u:p@h:notaport/db",  // 端口不是数字
            "mysql://u:p@h",              // 没库名
            "mysql://u:p@h/",             // 空库名
            "mysql://u:p@:3306/db",       // 空主机
        ] {
            assert!(parse_dsn(bad).is_err(), "{bad:?} 必须拒绝");
        }
    }

    #[test]
    fn backup_dir_defaults_under_upload_dir() {
        assert_eq!(backup_dir("uploads", ""), PathBuf::from("uploads/backups"));
        assert_eq!(backup_dir("uploads", "  "), PathBuf::from("uploads/backups"));
        assert_eq!(backup_dir("uploads", "/data/bak"), PathBuf::from("/data/bak"));
    }

    /// 保留策略：只认 backup_*.sql；多出来的删最旧；未超限不动；干扰文件不碰。
    #[test]
    fn prune_keeps_newest_and_ignores_others() {
        let dir = std::env::temp_dir().join(format!("bee_bk_prune_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |n: &str| std::fs::write(dir.join(n), "x").unwrap();
        for i in 1..=5 {
            mk(&format!("backup_2026010{i}_000000.sql"));
        }
        mk("backup_20260106_000000.sql.part"); // 中断残留：不是 .sql，不算也不删
        mk("notes.txt");
        std::fs::create_dir_all(dir.join("backup_20260107_000000.sql")).unwrap(); // 同名目录：不碰

        // 未超限：一个都不删
        assert_eq!(prune(&dir, 7).unwrap(), 0);
        assert_eq!(prune(&dir, 5).unwrap(), 0);
        // 保 3 份：删最旧的 2 份（01、02）
        assert_eq!(prune(&dir, 3).unwrap(), 2);
        assert!(!dir.join("backup_20260101_000000.sql").exists());
        assert!(!dir.join("backup_20260102_000000.sql").exists());
        assert!(dir.join("backup_20260103_000000.sql").exists());
        assert!(dir.join("backup_20260105_000000.sql").exists());
        assert!(dir.join("notes.txt").exists());
        assert!(dir.join("backup_20260106_000000.sql.part").exists());
        // keep=0：清光 .sql（配置层拒绝了 0，这里钉住函数语义：不删非 .sql）
        assert_eq!(prune(&dir, 0).unwrap(), 3);
        assert!(dir.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// stderr 尾巴：超长只留结尾；不是按字节切（多字节字符不炸）。
    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail("  hi\n"), "hi");
        let long: String = (0..300).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        assert_eq!(tail(&long).chars().count(), 200);
        assert!(tail(&long).ends_with(&long[long.len() - 10..]));
    }
}
