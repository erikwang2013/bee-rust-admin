// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 附件列表的 `entity_label` 不能绕过公告可见性：只有 `system:notice:list`
//! （或超管）能通过附件行看到**草稿**公告的标题；已发布公告的标题照常展示。
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

/// 建角色（授予给定菜单）→ 建用户 → 登录；返回 token。
async fn user_with_menus(
    c: &reqwest::Client,
    base: &str,
    admin: &str,
    name: &str,
    menu_ids: &[&str],
    data_scope: i8,
) -> String {
    let api = |p: &str| format!("{base}/api/v1{p}");
    let (st, v) = call(
        c,
        Method::POST,
        api("/roles"),
        Some(admin),
        Some(json!({"name": name, "code": name, "data_scope": data_scope, "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "建角色 {name}: {v}");
    let role = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        c,
        Method::PUT,
        api(&format!("/roles/{role}/menus")),
        Some(admin),
        Some(json!({"menu_ids": menu_ids})),
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
async fn attach_list_hides_draft_notice_title() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let c = reqwest::Client::new();
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");
    let admin_num = common::dec_id(&common::my_id(&base, &admin).await);

    // ── 一条已发布 + 一条草稿 ──────────────────────────────────
    let (st, v) = call(
        &c,
        Method::POST,
        api("/notices"),
        Some(&admin),
        Some(json!({"title": "已发布配图", "content": "公开正文", "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "发公告: {v}");
    let published = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::POST,
        api("/notices"),
        Some(&admin),
        Some(json!({"title": "草稿配图", "content": "还没发布", "status": 0})),
    )
    .await;
    assert_eq!(st, 200, "存草稿: {v}");
    let draft = common::as_id(&v["data"]["id"]);

    // 附件行直接入库：本用例只验列表展示，不需要真跑上传
    for (notice_id, file) in [
        (common::dec_id(&published), "published.png"),
        (common::dec_id(&draft), "draft.png"),
    ] {
        sqlx::query(
            "INSERT INTO attachment \
             (id, name, ext, mime, size, sha256, stored_path, admin_id, entity_type, entity_id, sort, created_at, updated_at) \
             VALUES (?, ?, 'png', 'image/png', 1, REPEAT('a', 64), 'attach_x_a.png', ?, 'notice', ?, 0, NOW(), NOW())",
        )
        .bind(common::new_row_id())
        .bind(file)
        .bind(admin_num)
        .bind(notice_id)
        .execute(&pool)
        .await
        .unwrap();
    }

    let row = |v: &Value, notice: &str| -> Value {
        v["data"]["list"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["entity_id"] == json!(notice))
            .unwrap_or_else(|| panic!("列表缺公告 {notice} 的附件: {v}"))
            .clone()
    };

    // 超管（is_super 视同有 system:notice:list）：草稿标题可见，但 is_public 仍是 false
    let (st, v) = call(&c, Method::GET, api("/attachments"), Some(&admin), None).await;
    assert_eq!(st, 200, "超管附件列表: {v}");
    assert_eq!(v["data"]["total"], 2, "{v}");
    assert_eq!(row(&v, &draft)["entity_label"], "草稿配图", "超管能看草稿标题: {v}");
    assert_eq!(row(&v, &draft)["is_public"], false, "草稿不是公开对象: {v}");
    assert_eq!(row(&v, &published)["entity_label"], "已发布配图", "{v}");
    assert_eq!(row(&v, &published)["is_public"], true, "{v}");

    // 只有附件列表权限（数据范围全部）：草稿标题必须为空，已发布标题照旧
    let (st, v) = call(&c, Method::GET, api("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200, "菜单树: {v}");
    let attach_list =
        find_perm(&v["data"], "system:attach:list").expect("种子里有 system:attach:list");
    let notice_list =
        find_perm(&v["data"], "system:notice:list").expect("种子里有 system:notice:list");
    let attach_only =
        user_with_menus(&c, &base, &admin, "attach_only", &[attach_list], 1).await;
    let (st, v) = call(&c, Method::GET, api("/attachments"), Some(&attach_only), None).await;
    assert_eq!(st, 200, "附件管理员列表: {v}");
    assert_eq!(v["data"]["total"], 2, "数据范围全部应看到两条: {v}");
    assert_eq!(
        row(&v, &draft)["entity_label"],
        "",
        "没有公告列表权限时不能泄露草稿标题: {v}"
    );
    assert_eq!(row(&v, &draft)["is_public"], false, "{v}");
    assert_eq!(row(&v, &published)["entity_label"], "已发布配图", "已发布标题保留: {v}");
    assert_eq!(row(&v, &published)["is_public"], true, "{v}");

    // 同时有 system:notice:list（非超管）：草稿标题恢复可见
    let attach_lister =
        user_with_menus(&c, &base, &admin, "attach_lister", &[attach_list, notice_list], 1).await;
    let (st, v) = call(&c, Method::GET, api("/attachments"), Some(&attach_lister), None).await;
    assert_eq!(st, 200, "公告管理员看附件列表: {v}");
    assert_eq!(row(&v, &draft)["entity_label"], "草稿配图", "有公告列表权限就能看: {v}");
    assert_eq!(row(&v, &draft)["is_public"], false, "可见标题不代表公开: {v}");
}
