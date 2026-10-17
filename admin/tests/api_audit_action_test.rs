// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 操作日志的 `action` 筛选：list / clear / export 共用 `filtered()`，trim 后精确匹配，
//! 空串/纯空白不过滤。起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use serde_json::Value;

/// 列表：返回 (total, rows)。
async fn list(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    params: &[(&str, &str)],
) -> (i64, Vec<Value>) {
    let r = c
        .get(format!("{base}/api/v1/audit-logs"))
        .query(params)
        .query(&[("size", "100")])
        .bearer_auth(token)
        .send()
        .await
        .expect("列表请求失败");
    let status = r.status().as_u16();
    let v: Value = r.json().await.expect("列表响应不是 JSON");
    assert_eq!(status, 200, "{v}");
    (
        v["data"]["total"].as_i64().unwrap_or(-1),
        v["data"]["list"].as_array().cloned().unwrap_or_default(),
    )
}

/// 导出：返回不含 BOM 的文本行（含表头）。
async fn csv_lines(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    params: &[(&str, &str)],
) -> Vec<String> {
    let r = c
        .get(format!("{base}/api/v1/audit-logs/export"))
        .query(params)
        .bearer_auth(token)
        .send()
        .await
        .expect("导出请求失败");
    assert_eq!(r.status(), 200, "导出必须 200");
    let body = r.bytes().await.unwrap();
    let text = String::from_utf8(body[3..].to_vec()).expect("BOM 之后是 UTF-8");
    text.trim_end().split("\r\n").map(str::to_string).collect()
}

/// 按筛选条件清空，返回被删条数。
async fn clear(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    params: &[(&str, &str)],
) -> i64 {
    let r = c
        .delete(format!("{base}/api/v1/audit-logs"))
        .query(params)
        .bearer_auth(token)
        .send()
        .await
        .expect("清空请求失败");
    let status = r.status().as_u16();
    let v: Value = r.json().await.expect("清空响应不是 JSON");
    assert_eq!(status, 200, "{v}");
    v["data"]["deleted"].as_i64().unwrap_or(-1)
}

#[tokio::test]
async fn audit_action_filter_is_exact_and_shared_by_list_clear_export() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let token = common::login(&base, "admin", "admin123").await.expect("admin 登录");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // 造数：模块单独用 probe，和中间件写的 auditlog 行隔离。
    // 两行「动作甲」（一成功一失败）+ 一行「动作乙」。
    for (action, status) in [("动作甲", 1), ("动作乙", 1), ("动作甲", 0)] {
        sqlx::query(
            "INSERT INTO audit_log \
             (id, admin_id, username, module, action, method, path, status, msg, changes, duration_ms, ip, created_at) \
             VALUES (?, 0, 'probe', 'probe', ?, 'POST', '/api/v1/probe', ?, '', '', 1, '127.0.0.1', '2026-01-01 00:00:00')",
        )
        .bind(common::new_row_id())
        .bind(action)
        .bind(status)
        .execute(&pool)
        .await
        .unwrap();
    }

    // ── list：精确匹配、trim、空串不过滤 ─────────────────────
    let (total, rows) = list(&c, &base, &token, &[("module", "probe"), ("action", "动作甲")]).await;
    assert_eq!(total, 2, "只回动作甲两行: {rows:?}");
    assert!(rows.iter().all(|r| r["action"] == "动作甲"), "{rows:?}");

    let (total, _) = list(&c, &base, &token, &[("module", "probe"), ("action", "动作")]).await;
    assert_eq!(total, 0, "前缀不算匹配（精确等号，不是模糊）");

    let (total, _) = list(&c, &base, &token, &[("module", "probe"), ("action", " 动作甲 ")]).await;
    assert_eq!(total, 2, "两侧空白被 trim");

    for empty in ["", "   "] {
        let (total, _) = list(&c, &base, &token, &[("module", "probe"), ("action", empty)]).await;
        assert_eq!(total, 3, "{empty:?} = 不过滤");
    }
    let (total, _) = list(&c, &base, &token, &[("module", "probe")]).await;
    assert_eq!(total, 3, "负控：不传 action 与传空串行数一致");

    // ── export：同一套筛选 ──────────────────────────────────
    let lines = csv_lines(&c, &base, &token, &[("module", "probe"), ("action", "动作甲")]).await;
    assert_eq!(lines.len(), 3, "表头 + 甲两行: {lines:?}");
    assert!(lines[1..].iter().all(|l| l.contains("动作甲")), "{lines:?}");

    let lines = csv_lines(&c, &base, &token, &[("module", "probe"), ("action", "")]).await;
    assert_eq!(lines.len(), 4, "负控：空串导出全量（表头 + 3 行）: {lines:?}");
    let lines = csv_lines(&c, &base, &token, &[("module", "probe")]).await;
    assert_eq!(lines.len(), 4, "负控：不传 action 与空串一致: {lines:?}");

    // ── clear：同一套筛选；不匹配的行一条不能少 ─────────────
    let deleted = clear(&c, &base, &token, &[("module", "probe"), ("action", "动作乙")]).await;
    assert_eq!(deleted, 1, "只删动作乙");
    let (total, rows) = list(&c, &base, &token, &[("module", "probe")]).await;
    assert_eq!(total, 2, "负控：动作甲两行还在: {rows:?}");
    assert!(rows.iter().all(|r| r["action"] == "动作甲"), "{rows:?}");

    let deleted = clear(&c, &base, &token, &[("module", "probe"), ("action", "")]).await;
    assert_eq!(deleted, 2, "空串 = 不过滤，剩下两行都删");
    let (total, _) = list(&c, &base, &token, &[("module", "probe")]).await;
    assert_eq!(total, 0, "probe 模块已清空");
}
