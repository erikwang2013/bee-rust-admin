// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! M4 全链路集成测试：数据权限 + 管理员/角色/菜单/部门/登录记录五个模块。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

/// 统一请求：返回 (状态码, JSON body)；断言消息带上 body，失败好定位。
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
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

#[tokio::test]
async fn full_admin_flow() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");

    // ── 1. 超管登录（顺带验证 AppJson：缺字段是 400 信封，不是 axum 的 422 纯文本）
    let (st, v) = call(&c, Method::POST, api("/auth/login"), None, Some(json!({"username": "admin"}))).await;
    assert_eq!(st, 400, "缺 password 应为 400: {v}");
    assert_eq!(v["code"], 400, "错误响应必须是 JSON 信封: {v}");

    let admin_token = common::login(&base, "admin", "admin123").await.expect("超管登录失败");

    // ── 2. 建部门「研发部」
    let (st, v) = call(&c, Method::POST, api("/depts"), Some(&admin_token),
        Some(json!({"parent_id": 0, "name": "研发部", "sort": 1}))).await;
    assert_eq!(st, 200, "建部门: {v}");
    let dept_id = v["data"]["id"].as_u64().expect("返回部门 id");

    // ── 3. 建角色（数据范围=本部门）并勾选「管理员管理」菜单
    let (st, v) = call(&c, Method::POST, api("/roles"), Some(&admin_token),
        Some(json!({"name": "运维", "code": "ops", "sort": 1, "data_scope": 3}))).await;
    assert_eq!(st, 200, "建角色: {v}");
    let role_id = v["data"]["id"].as_u64().expect("返回角色 id");

    let (st, v) = call(&c, Method::GET, api("/menus/tree"), Some(&admin_token), None).await;
    assert_eq!(st, 200, "菜单树: {v}");
    let root_menu_id = v["data"][0]["id"].as_u64().expect("根目录 id");
    let admin_menu_id = v["data"][0]["children"].as_array().expect("根目录有子菜单")
        .iter().find(|m| m["name"] == "管理员管理").expect("种子菜单含「管理员管理」")["id"]
        .as_u64().expect("菜单 id");

    let (st, v) = call(&c, Method::PUT, api(&format!("/roles/{role_id}/menus")), Some(&admin_token),
        Some(json!({"menu_ids": [admin_menu_id]}))).await;
    assert_eq!(st, 200, "勾选角色菜单: {v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/roles/{role_id}/menus")), Some(&admin_token), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"], json!([admin_menu_id]), "菜单勾选回读: {v}");

    // ── 4. 建管理员 op1（研发部 + 该角色）
    let (st, v) = call(&c, Method::POST, api("/admins"), Some(&admin_token), Some(json!({
        "username": "op1", "password": "op123456", "nickname": "操作员",
        "dept_id": dept_id, "role_ids": [role_id],
    }))).await;
    assert_eq!(st, 200, "建管理员: {v}");
    let op1_id = v["data"]["id"].as_u64().expect("返回管理员 id");

    // ── 5. op1：错密码 400（留一条失败记录）→ 正确登录 → 数据权限=本部门，只看到自己
    let (st, v) = call(&c, Method::POST, api("/auth/login"), None,
        Some(json!({"username": "op1", "password": "wrong"}))).await;
    assert_eq!(st, 400, "op1 错密码: {v}");
    let op1_token = common::login(&base, "op1", "op123456").await.expect("op1 登录失败");

    // 角色只勾了子菜单「管理员管理」→ 侧边栏必须带出父目录，否则拼不出树
    let (st, v) = call(&c, Method::GET, api("/auth/menus"), Some(&op1_token), None).await;
    assert_eq!(st, 200, "op1 菜单: {v}");
    assert_eq!(v["data"][0]["path"], "/system", "只勾子菜单也要补全父目录: {v}");
    assert_eq!(v["data"][0]["children"][0]["name"], "管理员管理", "子菜单在父目录下: {v}");

    let (st, v) = call(&c, Method::GET, api("/admins"), Some(&op1_token), None).await;
    assert_eq!(st, 200, "op1 看管理员列表: {v}");
    assert_eq!(v["data"]["total"], 1, "op1 只应看到自己: {v}");
    assert_eq!(v["data"]["list"][0]["username"], "op1");

    // op1 没有 remove / dept:add 权限
    let (st, v) = call(&c, Method::DELETE, api("/admins/1"), Some(&op1_token), None).await;
    assert_eq!(st, 403, "无 system:admin:remove 必须 403: {v}");
    let (st, v) = call(&c, Method::POST, api("/depts"), Some(&op1_token), Some(json!({"name": "偷偷建的"}))).await;
    assert_eq!(st, 403, "无 system:dept:add 必须 403: {v}");

    // ── 6. 超管禁用 op1 → 旧 token 立即失效（踢下线）
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{op1_id}/status")), Some(&admin_token),
        Some(json!({"status": 0}))).await;
    assert_eq!(st, 200, "禁用 op1: {v}");
    let (st, v) = call(&c, Method::GET, api("/auth/profile"), Some(&op1_token), None).await;
    assert_eq!(st, 401, "被禁用后旧 token 必须 401: {v}");

    // 角色仍被 op1 使用 → 不能删
    let (st, v) = call(&c, Method::DELETE, api(&format!("/roles/{role_id}")), Some(&admin_token), None).await;
    assert_eq!(st, 400, "角色被管理员使用不能删: {v}");

    // ── 7. 删 op1 → 列表不再包含
    let (st, v) = call(&c, Method::DELETE, api(&format!("/admins/{op1_id}")), Some(&admin_token), None).await;
    assert_eq!(st, 200, "删 op1: {v}");
    let (st, v) = call(&c, Method::GET, api("/admins"), Some(&admin_token), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["total"], 1, "只剩超管: {v}");
    assert_eq!(v["data"]["list"][0]["username"], "admin");

    // ── 8. 登录记录：查询 + 按条件清空（只删 op1 的）
    let (st, v) = call(&c, Method::GET, api("/login-logs"), Some(&admin_token), None).await;
    assert_eq!(st, 200, "登录记录: {v}");
    assert!(v["data"]["total"].as_u64().unwrap() >= 3,
        "至少 3 条（超管成功 / op1 失败 / op1 成功）: {v}");
    let (st, v) = call(&c, Method::DELETE, api("/login-logs?username=op1"), Some(&admin_token), None).await;
    assert_eq!(st, 200, "清空 op1 的记录: {v}");
    assert_eq!(v["data"]["deleted"], 2, "只删 op1 的两条: {v}");
    let (st, v) = call(&c, Method::GET, api("/login-logs"), Some(&admin_token), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["total"], 1, "剩下超管的记录: {v}");
    assert_eq!(v["data"]["list"][0]["username"], "admin");

    // 超长文本：400 信封（不是 MySQL 1406 的 500，也不是静默截断）
    let long_name = "角".repeat(300);
    let (st, v) = call(&c, Method::POST, api("/roles"), Some(&admin_token),
        Some(json!({"name": long_name, "code": "toolong", "data_scope": 1}))).await;
    assert_eq!(st, 400, "300 字角色名应 400: {v}");
    assert_eq!(v["code"], 400, "错误响应必须是 JSON 信封: {v}");

    // ── 9. 角色自定义数据范围：重复 id 去重 + 回读一致
    let (st, v) = call(&c, Method::PUT, api(&format!("/roles/{role_id}/depts")), Some(&admin_token),
        Some(json!({"dept_ids": [dept_id, dept_id]}))).await;
    assert_eq!(st, 200, "设置角色部门: {v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/roles/{role_id}/depts")), Some(&admin_token), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"], json!([dept_id]), "去重后回读一致: {v}");

    // ── 10. 菜单：建按钮 → 删掉；有子节点的目录不能删
    let (st, v) = call(&c, Method::POST, api("/menus"), Some(&admin_token), Some(json!({
        "parent_id": admin_menu_id, "name": "导出", "type": "F",
        "perm": "system:admin:export", "sort": 9,
    }))).await;
    assert_eq!(st, 200, "建按钮菜单: {v}");
    let btn_id = v["data"]["id"].as_u64().expect("返回菜单 id");
    let (st, v) = call(&c, Method::DELETE, api(&format!("/menus/{btn_id}")), Some(&admin_token), None).await;
    assert_eq!(st, 200, "删按钮菜单: {v}");
    let (st, v) = call(&c, Method::DELETE, api(&format!("/menus/{root_menu_id}")), Some(&admin_token), None).await;
    assert_eq!(st, 400, "有子节点的目录不能删: {v}");

    // ── 11. 角色不再被使用 → 可删
    let (st, v) = call(&c, Method::DELETE, api(&format!("/roles/{role_id}")), Some(&admin_token), None).await;
    assert_eq!(st, 200, "删角色: {v}");
    let (st, v) = call(&c, Method::GET, api("/roles"), Some(&admin_token), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["total"], 0, "角色已删: {v}");
}
