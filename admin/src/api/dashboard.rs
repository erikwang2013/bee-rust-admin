// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 真仪表盘（扩展工程项 2）：登录即可的聚合计数，**不挂权限码**。
//! 口径：只回计数与按天的登录次数（不含敏感明细）；全局口径，**不做数据范围过滤**
//! —— 数据范围管的是「谁能看哪些行」，仪表盘给的是「系统概览数字」，
//! 与操作日志的「谁登录过」明细不同，这里没有可泄露的单行。
use crate::auth::Auth;
use crate::error::{ApiError, ok};
use crate::models::{Admin, Dept, Notice, Role};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use bee_orm::{Model, Value as DbValue};
use chrono::{Duration, NaiveDate};
use serde_json::{Value, json};
use std::collections::HashMap;

/// 登录趋势窗口（含今天）：7 天。
const WINDOW_DAYS: i64 = 7;

/// 把按天聚合的结果补成**连续 7 天**（旧→新，没记录的日期补 0），
/// 同时算出窗口内的成功/失败总数。
///
/// 只按日期键查表：窗口外的行天然被忽略（SQL 侧也已 WHERE 截断，这里是双保险，
/// 保证「总数 = 序列之和」这条恒等式不会因为边界漂移而破）。
fn fill_days(found: &HashMap<String, (i64, i64)>, today: NaiveDate) -> (Vec<Value>, i64, i64) {
    let mut days = Vec::with_capacity(WINDOW_DAYS as usize);
    let (mut ok_total, mut fail_total) = (0i64, 0i64);
    for back in (0..WINDOW_DAYS).rev() {
        let key = (today - Duration::days(back)).format("%Y-%m-%d").to_string();
        let (success, fail) = found.get(&key).copied().unwrap_or((0, 0));
        ok_total += success;
        fail_total += fail;
        days.push(json!({ "date": key, "success": success, "fail": fail }));
    }
    (days, ok_total, fail_total)
}

/// 仪表盘统计。免权限码（登录即可）；`counts` 是四类基础计数，`logins` 是近 7 天序列 + 窗口内总数。
#[apidoc::title("仪表盘统计")]
#[apidoc::desc("登录即可（不挂权限码）。四类基础计数 + 近 7 天登录次数（按天，旧→新）与失败登录数；无数据的日期补 0")]
#[apidoc::url("/api/v1/dashboard/stats")]
#[apidoc::method("GET")]
#[apidoc::tag("系统")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(
    name = "data",
    ty = "object",
    desc = "{ counts: {admins, roles, depts, notices}, logins: {success_total, fail_total, days: [{date, success, fail} ×7]} }"
)]
pub async fn stats(State(state): State<AppState>, _auth: Auth) -> Result<Json<Value>, ApiError> {
    let admins = Admin::query().count(&state.db).await.map_err(ApiError::from)?;
    let roles = Role::query().count(&state.db).await.map_err(ApiError::from)?;
    let depts = Dept::query().count(&state.db).await.map_err(ApiError::from)?;
    let notices = Notice::query().count(&state.db).await.map_err(ApiError::from)?;

    let today = chrono::Local::now().date_naive();
    let start = (today - Duration::days(WINDOW_DAYS - 1))
        .and_hms_opt(0, 0, 0)
        .expect("0 点是合法时间");
    // COUNT(...) 而不是 SUM(...)：COUNT 回 BIGINT（as_i64 直读），SUM 在 MySQL 里回
    // DECIMAL，解码形状多一层不确定性。DATE_FORMAT 回字符串，同样避开 DATE 类型解码。
    let rows = state
        .db
        .query(
            "SELECT DATE_FORMAT(created_at, '%Y-%m-%d') AS d, \
                    COUNT(CASE WHEN status = 1 THEN 1 END) AS ok, \
                    COUNT(CASE WHEN status = 0 THEN 1 END) AS fail \
             FROM login_log WHERE created_at >= ? GROUP BY d",
            &[DbValue::from(start)],
        )
        .await
        .map_err(ApiError::from)?;

    let mut found: HashMap<String, (i64, i64)> = HashMap::new();
    for r in &rows {
        let Some(d) = r.get("d").and_then(|v| v.as_str()) else { continue };
        let ok_n = r.get("ok").and_then(|v| v.as_i64()).unwrap_or(0);
        let fail_n = r.get("fail").and_then(|v| v.as_i64()).unwrap_or(0);
        found.insert(d.to_string(), (ok_n, fail_n));
    }
    let (days, success_total, fail_total) = fill_days(&found, today);

    Ok(ok(json!({
        "counts": { "admins": admins, "roles": roles, "depts": depts, "notices": notices },
        "logins": { "success_total": success_total, "fail_total": fail_total, "days": days },
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 补 0、排序（旧→新）与「总数只算窗口内」三条性质。
    #[test]
    fn fill_days_pads_zero_and_sums_window_only() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let found = HashMap::from([
            ("2026-10-13".to_string(), (2, 1)),
            ("2026-10-15".to_string(), (1, 0)),
            ("2026-10-01".to_string(), (9, 9)), // 窗口外：即使出现在表里也不进序列/总数
        ]);
        let (days, ok_total, fail_total) = fill_days(&found, today);

        assert_eq!(days.len(), 7, "恒为 7 天");
        assert_eq!(days[0]["date"], "2026-10-10", "最早一天 = today - 6");
        assert_eq!(days[6]["date"], "2026-10-16", "最后一天 = today");
        assert_eq!(days[3]["success"], 2, "3 天前有数");
        assert_eq!(days[3]["fail"], 1);
        assert_eq!(days[5]["success"], 1, "昨天有数");
        assert_eq!(days[0]["success"], 0, "空日期补 0");
        assert_eq!(days[0]["fail"], 0);
        assert_eq!((ok_total, fail_total), (3, 1), "总数 = 窗口内之和（10-01 的 9/9 不算）");
    }

    /// 全空也要产出 7 条 0（前端空态的输入）。
    #[test]
    fn fill_days_empty_input_is_all_zero() {
        let today = NaiveDate::from_ymd_opt(2026, 1, 3).unwrap();
        let (days, ok_total, fail_total) = fill_days(&HashMap::new(), today);
        assert_eq!(days.len(), 7);
        assert_eq!(ok_total, 0);
        assert_eq!(fail_total, 0);
        // 跨月边界：2025-12-28 .. 2026-01-03
        assert_eq!(days[0]["date"], "2025-12-28");
        assert_eq!(days[6]["date"], "2026-01-03");
    }
}
