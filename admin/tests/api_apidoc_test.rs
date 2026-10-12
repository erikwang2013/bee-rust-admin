// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! apidoc 注解契约：menu / dept 的请求体字段必须与 DTO 逐字一致。
//!
//! 这两个模块在 v1.5「阶段 5b」漏了 param/returned（其余 9 个模块都补了），
//! 表现在 `/apidoc/api.json` 里它们的 POST/PUT **请求体是空数组** —— 对接方看不到要传什么。
//! 本文件把「文档里的字段」钉在代码上：注解漏一个字段 / 写错名字就红。
//!
//! 起真实进程（把 `[apidoc] enabled = true` 临时打进配置）+ 真库，需要 `BEE_ADMIN_DB_DSN`。
mod common;

use serde_json::{Value, json};

/// 文档站口令：只活在本测试进程的临时配置里，不进仓库配置。
const APIDOC_PW: &str = "apidoc-test-pw";

/// 在 api.json 里按 method + url 定位端点。
fn endpoint<'a>(doc: &'a Value, method: &str, url: &str) -> &'a Value {
    doc["endpoints"]
        .as_array()
        .unwrap_or_else(|| panic!("api.json 没有 endpoints 数组"))
        .iter()
        .find(|e| e["method"] == method && e["url"] == url)
        .unwrap_or_else(|| panic!("api.json 里找不到 {method} {url}"))
}

/// 端点的请求体字段 → `[{name, type, required}]`（保持注解顺序 = DTO 字段顺序）。
fn params(ep: &Value) -> Value {
    Value::Array(
        ep["params"]
            .as_array()
            .unwrap_or_else(|| panic!("端点没有 params 数组: {ep}"))
            .iter()
            .map(|p| {
                json!({
                    "name": p["name"],
                    "type": p["type"],
                    "required": p["required"].as_bool().unwrap_or(false),
                })
            })
            .collect(),
    )
}

/// 树形响应 `returned[0]`（data）的 children：节点字段名，按注解顺序。
fn node_fields(ep: &Value) -> Vec<String> {
    ep["returned"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("树形响应没有 children 节点: {ep}"))
        .iter()
        .map(|f| f["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn apidoc_menu_dept_bodies_are_documented() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    // 配置在启动时读取：要开文档站，必须换一个进程（start_server_conf 就是干这个的）
    let (_server, base) = common::start_server_conf(|cfg| {
        format!("{cfg}\n[apidoc]\nenabled = true\npassword = {APIDOC_PW}\n")
    })
    .await;
    let c = reqwest::Client::new();

    // 口令换 token：apidoc 2.x 起是 POST + JSON body 传**明文**口令
    // （不走 URL，且插件明确不做客户端摘要——摘要当凭据等于把摘要变成密码）。
    let v: Value = c
        .post(format!("{base}/apidoc/auth"))
        .json(&json!({ "password": APIDOC_PW }))
        .send()
        .await
        .expect("apidoc auth 请求失败")
        .json()
        .await
        .expect("apidoc auth 不是 JSON");
    let token = v["token"].as_str().unwrap_or_else(|| panic!("换 token 失败: {v}")).to_string();

    let doc: Value = c
        .get(format!("{base}/apidoc/api.json"))
        .query(&[("token", token.as_str())])
        .send()
        .await
        .expect("api.json 请求失败")
        .json()
        .await
        .expect("api.json 不是 JSON");

    // ── menu：请求体逐字等于 MenuBody（含 serde rename 后的 type）──
    let menu_fields = json!([
        {"name": "parent_id", "type": "string", "required": false},
        {"name": "name", "type": "string", "required": true},
        {"name": "type", "type": "string", "required": true},
        {"name": "perm", "type": "string", "required": false},
        {"name": "path", "type": "string", "required": false},
        {"name": "component", "type": "string", "required": false},
        {"name": "icon", "type": "string", "required": false},
        {"name": "sort", "type": "int", "required": false},
        {"name": "visible", "type": "int", "required": false},
        {"name": "status", "type": "int", "required": false},
    ]);
    assert_eq!(
        params(endpoint(&doc, "POST", "/api/v1/menus")),
        menu_fields,
        "POST /menus 的请求体注解与 MenuBody 不一致"
    );
    assert_eq!(
        params(endpoint(&doc, "PUT", "/api/v1/menus/{id}")),
        menu_fields,
        "PUT /menus/{{id}} 与 create 共用 MenuBody，字段必须一致"
    );

    // ── dept：同理 ──
    let dept_fields = json!([
        {"name": "parent_id", "type": "string", "required": false},
        {"name": "name", "type": "string", "required": true},
        {"name": "sort", "type": "int", "required": false},
        {"name": "leader", "type": "string", "required": false},
        {"name": "phone", "type": "string", "required": false},
        {"name": "status", "type": "int", "required": false},
    ]);
    assert_eq!(
        params(endpoint(&doc, "POST", "/api/v1/depts")),
        dept_fields,
        "POST /depts 的请求体注解与 DeptBody 不一致"
    );
    assert_eq!(
        params(endpoint(&doc, "PUT", "/api/v1/depts/{id}")),
        dept_fields,
        "PUT /depts/{{id}} 与 create 共用 DeptBody，字段必须一致"
    );

    // ── 两个树的响应结构：节点字段 + `children`（递归那一环不能漏）──
    assert_eq!(
        node_fields(endpoint(&doc, "GET", "/api/v1/menus/tree")),
        [
            "id", "parent_id", "name", "type", "perm", "path", "component", "icon", "sort",
            "visible", "status", "created_at", "updated_at", "children"
        ],
        "GET /menus/tree 的节点字段与 Menu 模型不一致"
    );
    assert_eq!(
        node_fields(endpoint(&doc, "GET", "/api/v1/depts/tree")),
        [
            "id", "parent_id", "name", "sort", "leader", "phone", "status", "created_at",
            "updated_at", "children"
        ],
        "GET /depts/tree 的节点字段与 Dept 模型不一致"
    );
}
