// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 日志保留策略（A4）：按 `[log] retain_days` 清理超期的 `login_log` / `audit_log` / `job_log`。
//! v1.6 起 `purge_all` 也由 `log_retention` 任务按天调用（启动时仍先清一遍，见 main）。
use bee_orm::pool::mysql::Pool;
use bee_orm::{OrmError, Value};
use chrono::NaiveDateTime;

/// 每批删除行数。一次删太多会长时间持锁堵住写入，分批删即便有个大积压也只短锁。
const BATCH: i64 = 5000;

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

#[cfg(test)]
mod tests {
    use super::*;

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
