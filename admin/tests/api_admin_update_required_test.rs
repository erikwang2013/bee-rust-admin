// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 修复 2：PUT /admins/{id} 必须提交完整更新体。部分 PUT 在反序列化阶段 400，
//! 不能靠 serde 默认值把 status 改回 1 / 把 role_ids 清空 / 把 dept_id 清零。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

async fn call(
    c: &reqwest::Client,
    method: Method,
    url: String,
    token: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut rb = c.request(method, url).bearer_auth(token);
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    let r = rb.send().await.expect("请求发送失败");
    (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn partial_admin_put_is_rejected_and_cannot_change_state() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let c = reqwest::Client::new();
    let token = common::login(&base, "admin", "admin123").await.expect("超管登录");

    // 建一个真实角色，用来钉「部分 PUT 不能清空角色」
    let (st, v) = call(
        &c,
        Method::POST,
        api("/roles"),
        &token,
        Some(json!({"name": "更新测试角色", "code": "gate_update_role", "data_scope": 4})),
    )
    .await;
    assert_eq!(st, 200, "建角色: {v}");
    let role = common::as_id(&v["data"]["id"]);

    let (st, v) = call(
        &c,
        Method::POST,
        api("/admins"),
        &token,
        Some(json!({
            "username": "gate_update_target",
            "password": "target12345",
            "nickname": "初始昵称",
            "dept_id": 0,
            "role_ids": [role],
        })),
    )
    .await;
    assert_eq!(st, 200, "建目标管理员: {v}");
    let id = common::as_id(&v["data"]["id"]);

    // ── 部分 PUT 一律 400，且库里的状态 / 角色原封不动 ──────────────────
    for partial in [
        json!({"status": 0}),
        json!({"role_ids": []}),
        json!({"nickname": "只改昵称"}),
    ] {
        let (st, v) = call(
            &c,
            Method::PUT,
            api(&format!("/admins/{id}")),
            &token,
            Some(partial.clone()),
        )
        .await;
        assert_eq!(st, 400, "缺字段的 PUT 必须 4xx: {partial} → {v}");
    }

    let (st, v) = call(&c, Method::GET, api(&format!("/admins/{id}")), &token, None).await;
    assert_eq!(st, 200, "详情: {v}");
    assert_eq!(v["data"]["status"], 1, "部分 PUT 不能把账号改成禁用/启用");
    assert_eq!(v["data"]["nickname"], "初始昵称", "部分 PUT 不能改任何列");
    let roles: Vec<String> = v["data"]["role_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| common::as_id(x))
        .collect();
    assert_eq!(roles, vec![role.clone()], "部分 PUT 不能清空角色");

    // ── 完整 PUT 照常工作（包括显式禁用 / 启用） ────────────────────────
    let full = |status: i8| {
        json!({
            "nickname": "完整更新",
            "email": "",
            "phone": "",
            "sex": 1,
            "dept_id": 0,
            "status": status,
            "remark": "",
            "role_ids": [role.clone()],
        })
    };
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{id}")), &token, Some(full(1))).await;
    assert_eq!(st, 200, "完整 PUT: {v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/admins/{id}")), &token, None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["nickname"], "完整更新");

    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{id}")), &token, Some(full(0))).await;
    assert_eq!(st, 200, "显式禁用: {v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/admins/{id}")), &token, None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["status"], 0, "完整 PUT 带 status=0 才能禁用");

    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{id}")), &token, Some(full(1))).await;
    assert_eq!(st, 200, "显式启用: {v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/admins/{id}")), &token, None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["status"], 1);
}
