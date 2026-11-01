// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! `/api/v2` 公告列表/未读只回摘要（正文按详情取），草稿详情不向无列表权限的人披露，
//! 发布/取消发布走窄接口 `PUT /notices/{id}/status`；
//! `/api/v1` 保持完整行契约（列表/未读含正文），验证破坏性变更没有漏进旧版本。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

async fn call(
    c: &reqwest::Client,
    method: Method,
    url: String,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let mut rb = c.request(method, url);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    let r = rb.send().await.expect("请求发送失败");
    (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
}

fn find_perm<'a>(menus: &'a Value, perm: &str) -> Option<&'a str> {
    for m in menus.as_array().into_iter().flatten() {
        if m["perm"].as_str() == Some(perm) {
            return m["id"].as_str();
        }
        if let Some(id) = find_perm(&m["children"], perm) {
            return Some(id);
        }
    }
    None
}

/// 建角色 → 勾一个权限码 → 建用户 → 登录；返回 token。
async fn user_with_perm(
    c: &reqwest::Client,
    api: impl Fn(&str) -> String,
    base: &str,
    admin: &str,
    name: &str,
    menu_id: &str,
) -> String {
    let (st, v) = call(
        c,
        Method::POST,
        api("/roles"),
        Some(admin),
        Some(json!({"name": name, "code": name, "data_scope": 4, "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "建角色 {name}: {v}");
    let role = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        c,
        Method::PUT,
        api(&format!("/roles/{role}/menus")),
        Some(admin),
        Some(json!({"menu_ids": [menu_id]})),
    )
    .await;
    assert_eq!(st, 200, "给 {name} 勾权限: {v}");
    let (st, v) = call(
        c,
        Method::POST,
        api("/admins"),
        Some(admin),
        Some(json!({"username": name, "password": "pass12345", "role_ids": [role]})),
    )
    .await;
    assert_eq!(st, 200, "建用户 {name}: {v}");
    common::login(base, name, "pass12345").await.unwrap_or_else(|| panic!("{name} 登录"))
}

#[tokio::test]
async fn notice_summary_detail_and_status() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let v1 = |p: &str| format!("{base}/api/v1{p}");
    let api = |p: &str| format!("{base}/api/v2{p}");
    let c = reqwest::Client::new();
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");

    // ── 一条已发布 + 一条草稿 ──────────────────────────────────
    let (st, v) = call(
        &c,
        Method::POST,
        api("/notices"),
        Some(&admin),
        Some(json!({"title": "系统维护", "content": "今晚 22:00 停机维护\n请提前保存工作", "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "发公告: {v}");
    let n1 = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::POST,
        api("/notices"),
        Some(&admin),
        Some(json!({"title": "草稿箱", "content": "还没想好", "status": 0})),
    )
    .await;
    assert_eq!(st, 200, "存草稿: {v}");
    let n2 = common::as_id(&v["data"]["id"]);

    // ── 管理列表：摘要不带正文，筛选/分页/filters 语义不变 ─────
    let (st, v) = call(&c, Method::GET, api("/notices"), Some(&admin), None).await;
    assert_eq!(st, 200, "公告列表: {v}");
    assert_eq!(v["data"]["total"], 2, "{v}");
    let rows = v["data"]["list"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{v}");
    for row in rows {
        assert!(row.get("content").is_none(), "列表行不能带 content: {row}");
        assert!(row.get("created_by").is_none(), "列表行是摘要，不带 created_by: {row}");
        for k in ["id", "title", "status", "published_at", "created_at", "updated_at"] {
            assert!(row.get(k).is_some(), "摘要缺字段 {k}: {row}");
        }
    }
    let (st, v) = call(&c, Method::GET, api("/notices?status=0"), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 1, "状态筛选不变: {v}");
    assert_eq!(v["data"]["list"][0]["id"], json!(n2), "{v}");

    // v1 兼容契约：列表仍是完整行（含正文 / created_by），不随 v2 摘要化
    let (st, v) = call(&c, Method::GET, v1("/notices"), Some(&admin), None).await;
    assert_eq!(st, 200, "v1 公告列表: {v}");
    assert_eq!(v["data"]["total"], 2, "{v}");
    for row in v["data"]["list"].as_array().unwrap() {
        assert!(row.get("content").is_some(), "v1 列表保留正文: {row}");
        assert!(row.get("created_by").is_some(), "v1 列表保留 created_by: {row}");
    }

    // ── 普通用户（空权限）：未读只有已发布且不带正文 ───────────
    let (st, v) = call(
        &c,
        Method::POST,
        v1("/roles"),
        Some(&admin),
        Some(json!({"name": "notice_none", "code": "notice_none", "data_scope": 4, "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "建空角色: {v}");
    let empty_role = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::POST,
        v1("/admins"),
        Some(&admin),
        Some(
            json!({"username": "notice_plain", "password": "plain12345", "role_ids": [empty_role]}),
        ),
    )
    .await;
    assert_eq!(st, 200, "建普通用户: {v}");
    let plain = common::login(&base, "notice_plain", "plain12345").await.expect("普通用户登录");

    let (st, v) = call(&c, Method::GET, api("/notices/unread"), Some(&plain), None).await;
    assert_eq!(st, 200, "未读: {v}");
    assert_eq!(v["data"]["total"], 1, "草稿不进未读: {v}");
    let unread = v["data"]["list"].as_array().unwrap();
    assert_eq!(unread.len(), 1, "{v}");
    assert_eq!(unread[0]["id"], json!(n1), "{v}");
    assert!(unread[0].get("content").is_none(), "未读行不能带 content: {v}");
    for k in ["id", "title", "status", "published_at", "created_at", "updated_at"] {
        assert!(unread[0].get(k).is_some(), "未读摘要缺字段 {k}: {v}");
    }

    // v1 未读仍是完整行，外部调用方不会被这次摘要化打断
    let (st, v) = call(&c, Method::GET, v1("/notices/unread"), Some(&plain), None).await;
    assert_eq!(st, 200, "v1 未读: {v}");
    assert_eq!(v["data"]["total"], 1, "{v}");
    assert_eq!(v["data"]["list"][0]["content"], "今晚 22:00 停机维护\n请提前保存工作", "{v}");

    // ── 详情：已发布登录即可；草稿只对有 list 权限的人可见 ─────
    let (st, v) = call(&c, Method::GET, api(&format!("/notices/{n1}")), Some(&plain), None).await;
    assert_eq!(st, 200, "已发布详情: {v}");
    assert_eq!(v["data"]["content"], "今晚 22:00 停机维护\n请提前保存工作", "{v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/notices/{n2}")), Some(&plain), None).await;
    assert_eq!(st, 404, "草稿对无权限用户按不存在处理（不泄露存在性）: {v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/notices/{n2}")), Some(&admin), None).await;
    assert_eq!(st, 200, "超管能看草稿详情: {v}");
    assert_eq!(v["data"]["content"], "还没想好", "{v}");
    let (st, v) = call(
        &c,
        Method::GET,
        api(&format!("/notices/{}", common::enc_id(999_999))),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(st, 404, "不存在的公告: {v}");
    let (st, _v) = call(&c, Method::GET, api(&format!("/notices/{n1}")), None, None).await;
    assert_eq!(st, 401, "详情要登录");

    // 非超管但挂了 system:notice:list：草稿详情可见
    let (st, v) = call(&c, Method::GET, v1("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200, "菜单树: {v}");
    let list_menu =
        find_perm(&v["data"], "system:notice:list").expect("种子里有 system:notice:list");
    let lister = user_with_perm(&c, &v1, &base, &admin, "notice_lister", list_menu).await;
    let (st, v) = call(&c, Method::GET, api(&format!("/notices/{n2}")), Some(&lister), None).await;
    assert_eq!(st, 200, "有 list 权限（非超管）能看草稿详情: {v}");
    assert_eq!(v["data"]["content"], "还没想好", "{v}");
    let (st, v) = call(&c, Method::GET, api("/notices"), Some(&lister), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 2, "列表权限不变: {v}");

    // ── 状态窄接口：权限、发布/撤回语义、审计 ──────────────────
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/notices/{n2}/status")),
        Some(&plain),
        Some(json!({"status": 1})),
    )
    .await;
    assert_eq!(st, 403, "没有 system:notice:edit 不能发布: {v}");

    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/notices/{n2}/status")),
        Some(&admin),
        Some(json!({"status": 1})),
    )
    .await;
    assert_eq!(st, 200, "发布草稿: {v}");
    let first_publish: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT published_at FROM notice WHERE id = ?")
            .bind(common::dec_id(&n2))
            .fetch_one(&pool)
            .await
            .unwrap();
    let skew = (first_publish - chrono::Local::now().naive_local()).num_seconds();
    assert!(skew.abs() <= 1, "首次发布写当下时间，实际 {first_publish}");

    // 发布后进未读，仍然是摘要
    let (st, v) = call(&c, Method::GET, api("/notices/unread"), Some(&plain), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 2, "新发布的草稿进未读: {v}");
    assert_eq!(v["data"]["list"][0]["id"], json!(n2), "按 id DESC: {v}");
    assert!(v["data"]["list"][0].get("content").is_none(), "未读仍不带 content: {v}");

    // 取消发布：published_at 留痕不清空，未读里消失
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/notices/{n2}/status")),
        Some(&admin),
        Some(json!({"status": 0})),
    )
    .await;
    assert_eq!(st, 200, "取消发布: {v}");
    let withdrawn: Option<chrono::NaiveDateTime> =
        sqlx::query_scalar("SELECT published_at FROM notice WHERE id = ?")
            .bind(common::dec_id(&n2))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(withdrawn, Some(first_publish), "撤回不清 published_at（留痕）");
    let (st, v) = call(&c, Method::GET, api("/notices/unread"), Some(&plain), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 1, "撤回后不再进未读: {v}");
    assert_eq!(v["data"]["list"][0]["id"], json!(n1), "{v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/notices/{n2}")), Some(&plain), None).await;
    assert_eq!(st, 404, "撤回后对无权限用户又是 404: {v}");

    // 再发布：不重写首次发布时间
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/notices/{n2}/status")),
        Some(&admin),
        Some(json!({"status": 1})),
    )
    .await;
    assert_eq!(st, 200, "再发布: {v}");
    let republished: Option<chrono::NaiveDateTime> =
        sqlx::query_scalar("SELECT published_at FROM notice WHERE id = ?")
            .bind(common::dec_id(&n2))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(republished, Some(first_publish), "published_at 只在首次发布时写");

    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/notices/{}/status", common::enc_id(888_888))),
        Some(&admin),
        Some(json!({"status": 1})),
    )
    .await;
    assert_eq!(st, 404, "不存在的公告: {v}");

    // 状态接口与 update 一样进审计（module=notice）：403 + 发布 + 撤回 + 再发布 + 404
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE module = 'notice' AND action = '修改公告状态'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 5, "状态变更全部留痕: {n}");

    // v2 的其余公告别名也必须真实可达（同一 handler，路由不能漏挂）
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/notices/{n1}")),
        Some(&admin),
        Some(json!({
            "title": "系统维护",
            "content": "今晚 22:00 停机维护\n请提前保存工作",
            "status": 1,
        })),
    )
    .await;
    assert_eq!(st, 200, "v2 编辑公告: {v}");
    let (st, v) =
        call(&c, Method::POST, api(&format!("/notices/{n1}/read")), Some(&plain), None).await;
    assert_eq!(st, 200, "v2 标记已读: {v}");
    let (st, v) =
        call(&c, Method::DELETE, api(&format!("/notices/{n2}")), Some(&admin), None).await;
    assert_eq!(st, 200, "v2 删除公告: {v}");
}
