// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 保留策略（A4 / E2-2）：按 `[log] retain_days` 清理超期的 `login_log` / `audit_log` / `job_log`，
//! 按 `[auth] device_keep_days` 清理「已踢且久未露面」的登录设备行（`device_clean` 任务）。
//! v1.6 起 `purge_all` 也由 `log_retention` 任务按天调用（启动时仍先清一遍，见 main）。
use crate::state::AppState;
use bee_orm::pool::mysql::Pool;
use bee_orm::{OrmError, Value};
use chrono::NaiveDateTime;

/// 每批删除行数。一次删太多会长时间持锁堵住写入，分批删即便有个大积压也只短锁。
const BATCH: i64 = 5000;

/// 登录设备清理（E2-2）的表与条件。
/// Ruling：分派说明写的口径是 `updated_at`，但 `admin_login_device` **没有这一列**
/// （见 `models/login_device.rs`：id / admin_id / user_agent / ip / status / created_at /
/// last_active_at），所以「久未露面」按 `last_active_at` 算 —— 语义上也更贴
/// （踢出之后用户再没拿这台设备露过面）。文档与台账都按这个口径写。
/// 条件里的 `status = 0` 是安全线：在用的设备行（status=1）是登录凭据的一部分
/// （token 里带着它的 id），删了等于把在线的人直接踢下线 —— 本任务只清**已踢**的留痕。
const DEVICE_TABLE: &str = "admin_login_device";
const DEVICE_WHERE: &str = "status = 0 AND last_active_at < ?";

/// 保留窗口 → cutoff 时间串：`created_at < cutoff` 即过期。
/// `retain_days <= 0`（0 = 永久保留）时窗口为零，调用方本就不该调它。
pub fn cutoff(retain_days: i64, now: NaiveDateTime) -> String {
    (now - chrono::Duration::days(retain_days.max(0)))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// 删除 `table` 中 `col < cutoff` 的行，按 `LIMIT BATCH` 循环到删完；
/// 返回删除总行数。`table` / `col` 只传本模块写死的常量名（job_log 的时间列叫 started_at）。
pub async fn purge_before(db: &Pool, table: &str, col: &str, cutoff: &str) -> Result<i64, OrmError> {
    let mut total = 0i64;
    loop {
        let sql = format!("DELETE FROM {table} WHERE {col} < ? LIMIT {BATCH}");
        let n = db.execute(&sql, &[Value::from(cutoff)]).await?;
        total += n as i64;
        // 删满一批说明后面可能还有，继续；不满一批 = 删干净了
        if n < BATCH as u64 {
            return Ok(total);
        }
    }
}

/// 对保留策略覆盖的日志表各跑一遍。单表失败只告警不下沉（明天再试）。
pub async fn purge_all(db: &Pool, retain_days: i64) -> i64 {
    if retain_days <= 0 {
        return 0; // 0 = 永久保留
    }
    let cutoff = cutoff(retain_days, crate::util::now());
    let mut total = 0;
    // job_log 也按这个窗口清（并入 log_retention），否则执行记录无限涨
    for (table, col) in [("login_log", "created_at"), ("audit_log", "created_at"), ("job_log", "started_at")] {
        match purge_before(db, table, col, &cutoff).await {
            Ok(n) => {
                total += n as i64;
                tracing::info!("清理 {table} 过期日志 {n} 条（早于 {cutoff}）");
            }
            Err(e) => tracing::warn!("清理 {table} 失败: {e}"),
        }
    }
    total
}

/// 登录设备清理（`device_clean` 任务，每天一趟）：删「已踢（status=0）且 `[auth]
/// device_keep_days` 天没露面」的 `admin_login_device` 行。`days <= 0` = 永久保留，跳过。
/// 分批删的理由同 `purge_before`；单批失败直接返回 Err —— 这张表只有一个删除方，
/// 失败要让 job_log 上看得见（日志表那三张是各清各的，才用「告警不下沉」）。
pub async fn device_clean(state: AppState) -> Result<String, String> {
    let days = state.cfg.device_keep_days;
    if days <= 0 {
        return Ok("device_keep_days=0（永久保留），跳过".into());
    }
    let cutoff = cutoff(days, crate::util::now());
    let sql = format!("DELETE FROM {DEVICE_TABLE} WHERE {DEVICE_WHERE} LIMIT {BATCH}");
    let mut total = 0i64;
    loop {
        let n = state
            .db
            .execute(&sql, &[Value::from(cutoff.as_str())])
            .await
            .map_err(|e| format!("清理 {DEVICE_TABLE} 失败: {e}"))?;
        total += n as i64;
        if n < BATCH as u64 {
            break; // 不满一批 = 删干净了
        }
    }
    Ok(format!("清理过期登录设备 {total} 条（已踢且 {days} 天没露面）"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 设备清理的窗口口径（E2-2）：默认 30 天，0/负数 = 永久保留（跳过）。
    /// 删的条件里必须带 `status = 0` —— 在用的设备行是登录凭据的一部分（token 里带它的 id），
    /// 删了等于把在线的人踢掉。
    #[test]
    fn device_clean_window_and_predicate() {
        let now = at("2026-10-17 12:00:00");
        assert_eq!(cutoff(30, now), "2026-09-17 12:00:00", "默认 30 天：9 月中旬之前的算过期");
        assert_eq!(cutoff(0, now), "2026-10-17 12:00:00", "0 = 永久保留（调用方直接跳过）");
        assert!(DEVICE_WHERE.contains("status = 0"), "只清已踢的: {DEVICE_WHERE}");
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn cutoff_subtracts_window() {
        let now = at("2026-10-06 12:00:00");
        assert_eq!(cutoff(90, now), "2026-07-08 12:00:00");
        assert_eq!(cutoff(1, now), "2026-10-05 12:00:00");
        assert_eq!(cutoff(365, now), "2025-10-06 12:00:00");
        // 0/负值不倒退（`purge_all` 会直接跳过）
        assert_eq!(cutoff(0, now), "2026-10-06 12:00:00");
        assert_eq!(cutoff(-7, now), "2026-10-06 12:00:00");
    }
}
