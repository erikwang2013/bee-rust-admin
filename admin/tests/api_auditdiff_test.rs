// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 扩展工程 C1（项 8）审计差异（before/after）：admin / role / dept / dict 四个模块的
//! 编辑把「改了哪些字段、旧值→新值」落进 audit_log.changes。
//! 两条硬口径：①密文列（email/phone）只记「改过」，值（明文或密文）都不进摘要；
//! ②密码类接口不产生摘要（负控：摘要里既没有口令串、也没有任何字段）。
//! 起真实进程 + 真库（bee_admin_test），需要 `BEE_ADMIN_DB_DSN`。
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

/// 取最近一条某模块某动作的审计行（列表按 id DESC，取第一条命中即可）。
async fn last_audit(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    module: &str,
    action: &str,
) -> Value {
    let (st, v) = call(
        c,
        Method::GET,
        format!("{base}/api/v1/audit-logs?module={module}&size=50"),
        Some(token),
        None,
    )
    .await;
    assert_eq!(st, 200, "{v}");
    v["data"]["list"]
        .as_array()
        .expect("list 必须是数组")
        .iter()
        .find(|r| r["action"] == action)
        .unwrap_or_else(|| panic!("审计里没有「{action}」（module={module}）: {v}"))
        .clone()
}

/// 审计行的 changes 列 → JSON；空/非 JSON 一律当失败（摘要必须是 JSON 串）。
fn changes_of(row: &Value) -> Value {
    let raw = row["changes"]
        .as_str()
        .unwrap_or_else(|| panic!("changes 列缺失或不是字符串: {row}"));
    serde_json::from_str(raw).unwrap_or_else(|e| panic!("changes 不是合法 JSON（{e}）: {raw}"))
}

#[tokio::test]
async fn audit_changes_record_before_after_for_four_modules() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录失败");

    // ── 1. admin：改昵称 + 邮箱（密文列）→ 摘要记昵称旧→新、邮箱只记「改过」 ──
    let (st, v) = call(
        &c,
        Method::POST,
        api("/admins"),
        Some(&admin),
        Some(json!({
            "username": "diffee", "password": "diffee123", "nickname": "旧昵称",
            "email": "old@x.com", "phone": "13800000000"
        })),
    )
    .await;
    assert_eq!(st, 200, "造被改的管理员: {v}");
    let target = common::as_id(&v["data"]["id"]);

    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/admins/{target}")),
        Some(&admin),
        Some(json!({
            "nickname": "新昵称", "email": "new@x.com", "phone": "13800000000",
            "sex": 0, "dept_id": 0, "status": 1, "remark": "", "role_ids": []
        })),
    )
    .await;
    assert_eq!(st, 200, "改昵称与邮箱: {v}");

    let row = last_audit(&c, &base, &admin, "admin", "编辑管理员").await;
    let j = changes_of(&row);
    assert_eq!(j["nickname"]["old"], "旧昵称", "{j}");
    assert_eq!(j["nickname"]["new"], "新昵称", "{j}");
    assert_eq!(j["email"]["changed"], true, "密文列只记「改过」: {j}");
    let raw = row["changes"].as_str().unwrap_or("");
    assert!(!raw.contains("new@x.com"), "邮箱明文不进摘要: {raw}");
    assert!(!raw.contains("13800000000"), "没变的手机号也不进摘要: {raw}");
    assert!(j.get("phone").is_none(), "没改的字段不进摘要: {j}");
    assert!(j.get("sex").is_none(), "没改的字段不进摘要: {j}");

    // ── 2. 负控：重置密码 → 这一行不产生摘要（口令串绝不出现） ──
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/admins/{target}/password")),
        Some(&admin),
        Some(json!({ "password": "Diffpass123" })),
    )
    .await;
    assert_eq!(st, 200, "重置密码: {v}");
    let row = last_audit(&c, &base, &admin, "admin", "重置管理员密码").await;
    let raw = row["changes"].as_str().unwrap_or("");
    assert!(raw.is_empty(), "密码接口不产生摘要: {raw}");
    assert!(raw != "{\"password\":{\"changed\":true}}", "更不许把字段名写成 password: {raw}");

    // ── 3. role：改备注 → 摘要里 remark 旧→新 ──
    let (st, v) = call(
        &c,
        Method::POST,
        api("/roles"),
        Some(&admin),
        Some(json!({"name": "差异角色", "code": "diff_role", "data_scope": 1, "remark": "旧备注"})),
    )
    .await;
    assert_eq!(st, 200, "建角色: {v}");
    let role = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/roles/{role}")),
        Some(&admin),
        Some(json!({"name": "差异角色", "code": "diff_role", "data_scope": 1, "remark": "新备注"})),
    )
    .await;
    assert_eq!(st, 200, "改角色: {v}");
    let j = changes_of(&last_audit(&c, &base, &admin, "role", "编辑角色").await);
    assert_eq!(j["remark"]["old"], "旧备注", "{j}");
    assert_eq!(j["remark"]["new"], "新备注", "{j}");

    // ── 4. dept：改名字 → 摘要记 name 旧→新 ──
    let (st, v) = call(
        &c,
        Method::POST,
        api("/depts"),
        Some(&admin),
        Some(json!({"name": "差异部门", "leader": "老王"})),
    )
    .await;
    assert_eq!(st, 200, "建部门: {v}");
    let dept = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/depts/{dept}")),
        Some(&admin),
        Some(json!({"name": "差异部门改", "leader": "老王", "phone": "13800000001", "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "改部门: {v}");
    let j = changes_of(&last_audit(&c, &base, &admin, "dept", "编辑部门").await);
    assert_eq!(j["name"]["old"], "差异部门", "{j}");
    assert_eq!(j["name"]["new"], "差异部门改", "{j}");
    assert_eq!(j["phone"]["old"], "", "新建时没填电话，旧值空串: {j}");

    // ── 5. dict：类型与字典项各改一处，两条摘要都要有 ──
    let (st, v) = call(
        &c,
        Method::POST,
        api("/dicts"),
        Some(&admin),
        Some(json!({"name": "差异字典", "code": "diff_dict", "remark": "旧"})),
    )
    .await;
    assert_eq!(st, 200, "建字典类型: {v}");
    let dt = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::POST,
        api("/dict-items"),
        Some(&admin),
        Some(json!({"type_code": "diff_dict", "label": "旧项", "value": "a"})),
    )
    .await;
    assert_eq!(st, 200, "建字典项: {v}");
    let di = common::as_id(&v["data"]["id"]);

    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/dicts/{dt}")),
        Some(&admin),
        Some(json!({"name": "差异字典", "status": 1, "remark": "新"})),
    )
    .await;
    assert_eq!(st, 200, "改字典类型: {v}");
    let j = changes_of(&last_audit(&c, &base, &admin, "dict", "编辑字典类型").await);
    assert_eq!(j["remark"]["old"], "旧", "{j}");
    assert_eq!(j["remark"]["new"], "新", "{j}");

    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/dict-items/{di}")),
        Some(&admin),
        Some(json!({"label": "新项", "value": "a"})),
    )
    .await;
    assert_eq!(st, 200, "改字典项: {v}");
    let j = changes_of(&last_audit(&c, &base, &admin, "dict", "编辑字典项").await);
    assert_eq!(j["label"]["old"], "旧项", "{j}");
    assert_eq!(j["label"]["new"], "新项", "{j}");
}
