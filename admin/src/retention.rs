// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 日志保留策略（A4）：按 `[log] retain_days` 清理超期的 `login_log` / `audit_log`。
use bee_orm::{Db, OrmError};
use chrono::NaiveDateTime;

/// 每批删除行数。一次删太多会长时间持锁堵住写入，分批删即便有个大积压也只短锁。
const BATCH: u64 = 5000;

/// 保留窗口 → cutoff 时间串：`created_at < cutoff` 即过期。
/// `retain_days <= 0`（0 = 永久保留）时窗口为零，调用方本就不该调它。
pub fn cutoff(retain_days: i64, now: NaiveDateTime) -> String {
    (now - chrono::Duration::days(retain_days.max(0)))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// 删除 `table` 中 `created_at < cutoff` 的行，按 `LIMIT BATCH` 循环到删完；
/// 返回删除总行数。`table` 只传本模块写死的常量名。
pub async fn purge_before(db: &Db, table: &str, cutoff: &str) -> Result<u64, OrmError> {
    let mut total = 0u64;
    loop {
        let sql = format!("DELETE FROM {table} WHERE created_at < ? LIMIT {BATCH}");
        let n = sqlx::query(&sql)
            .bind(cutoff)
            .execute(db.pool())
            .await
            .map_err(OrmError::from)?
            .rows_affected();
        total += n;
        // 删满一批说明后面可能还有，继续；不满一批 = 删干净了
        if n < BATCH {
            return Ok(total);
        }
    }
}

/// 对保留策略覆盖的日志表各跑一遍。单表失败只告警不下沉（明天再试）。
pub async fn purge_all(db: &Db, retain_days: i64) -> u64 {
    if retain_days <= 0 {
        return 0; // 0 = 永久保留
    }
    let cutoff = cutoff(retain_days, crate::util::now());
    let mut total = 0;
    for table in ["login_log", "audit_log"] {
        match purge_before(db, table, &cutoff).await {
            Ok(n) => {
                total += n;
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
