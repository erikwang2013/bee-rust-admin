// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! BRA v1.7 C3 后端：统一信封的稳定业务错误码 `err` 与参数 `args`。
//! 契约（错误码表）见 `docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md`「C3 契约」。
//! 这里钉的是信封形状 + 几个代表码（含三个带参码），逐模块的行为仍归各自的 `api_v1x_test.rs`。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

/// 统一请求：返回 (状态码, JSON body)。
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

/// 登录并指定 `X-Real-IP`：限流按 IP 分桶，换个 IP 就是换个桶。
async fn login_from(c: &reqwest::Client, url: &str, ip: &str, user: &str, pass: &str) -> (u16, Value) {
    let r = c
        .post(url)
        .header("X-Real-IP", ip)
        .json(&json!({"username": user, "password": pass}))
        .send()
        .await
        .expect("请求发送失败");
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

/// 静态码（表里 args 列为空的）：`err` 逐字命中、`msg` 中文原文仍在、**整个不带** `args` 键。
fn assert_static_err(v: &Value, err: &str) {
    assert_eq!(v["err"], err, "err 必须逐字等于契约里的键: {v}");
    assert!(!v["msg"].as_str().unwrap_or("").is_empty(), "msg 中文原文仍要保留: {v}");
    assert!(v.get("args").is_none(), "静态码不带 args 键: {v}");
}

/// 带参码：`err` 命中且 `args` 是对象；返回 `args` 供逐参数断言。
fn assert_args_err<'a>(v: &'a Value, err: &str) -> &'a Value {
    assert_eq!(v["err"], err, "err 必须逐字等于契约里的键: {v}");
    assert!(v["args"].is_object(), "带参码必须有 args 对象: {v}");
    &v["args"]
}

#[tokio::test]
async fn v17_err_envelope_contract() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── 成功响应没有 err/args（契约：err 只出现在错误上）──
    let (st, v) = call(
        &c,
        Method::POST,
        api("/auth/login"),
        None,
        Some(json!({"username": "admin", "password": "admin123"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert!(v.get("err").is_none() && v.get("args").is_none(), "成功信封不带 err/args: {v}");
    let admin = v["data"]["token"].as_str().expect("登录 token").to_string();

    // ── auth.* 静态码 ──
    let (st, v) = call(
        &c,
        Method::POST,
        api("/auth/login"),
        None,
        Some(json!({"username": "  ", "password": ""})),
    )
    .await;
    assert_eq!(st, 400, "空账号密码 400: {v}");
    assert_static_err(&v, "auth.empty_credentials");

    let (st, v) = call(
        &c,
        Method::POST,
        api("/auth/login"),
        None,
        Some(json!({"username": "v17_nobody", "password": "nope"})),
    )
    .await;
    assert_eq!(st, 400, "{v}");
    assert_static_err(&v, "auth.bad_credentials");

    let (st, v) = call(&c, Method::GET, api("/auth/profile"), None, None).await;
    assert_eq!(st, 401, "无 token: {v}");
    assert_static_err(&v, "auth.unauthorized");

    // ── 403 + args.code：权限不足（用户最常撞见的那个 403，按契约也能翻译）──
    let (st, v) = call(
        &c,
        Method::POST,
        api("/admins"),
        Some(&admin),
        Some(json!({"username": "v17_plain", "password": "plain123"})),
    )
    .await;
    assert_eq!(st, 200, "建一个不带角色的普通管理员: {v}");
    let (st, v) = call(
        &c,
        Method::POST,
        api("/auth/login"),
        None,
        Some(json!({"username": "v17_plain", "password": "plain123"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    let plain = v["data"]["token"].as_str().expect("普通管理员 token").to_string();

    let (st, v) = call(&c, Method::GET, api("/dicts"), Some(&plain), None).await;
    assert_eq!(st, 403, "{v}");
    let args = assert_args_err(&v, "auth.forbidden");
    assert_eq!(args["code"], "system:dict:list", "args.code 是缺的那个权限码原始值: {v}");
    assert!(v["msg"].as_str().unwrap_or("").starts_with("缺少权限"), "msg 仍是中文原文: {v}");

    // ── 带参：common.too_long（check_len 收字段键，args 是给前端拼文案的原始值）──
    let (st, v) = call(
        &c,
        Method::PUT,
        api("/auth/profile"),
        Some(&admin),
        Some(json!({"nickname": "x".repeat(65), "email": "", "phone": ""})),
    )
    .await;
    assert_eq!(st, 400, "{v}");
    assert_eq!(v["err"], "common.too_long", "{v}");
    assert_eq!(v["args"]["field"], "nickname", "args.field 传字段键，不是中文标签: {v}");
    assert_eq!(v["args"]["max"], 64, "{v}");
    assert_eq!(v["msg"], "昵称 长度不能超过 64 个字符", "msg 仍拼中文（日志/curl 读得懂）: {v}");

    // 同一个码、不同字段键：args 跟着字段走
    let (st, v) = call(
        &c,
        Method::PUT,
        api("/auth/profile"),
        Some(&admin),
        Some(json!({"nickname": "", "email": format!("{}@x.com", "y".repeat(130)), "phone": ""})),
    )
    .await;
    assert_eq!(st, 400, "{v}");
    assert_eq!(v["err"], "common.too_long", "{v}");
    assert_eq!(v["args"]["field"], "email", "{v}");
    assert_eq!(v["args"]["max"], 128, "{v}");

    // ── 框架层兜底：路由级 404 也带码；405 没有稳定码就整个不带 err ──
    let (st, v) = call(&c, Method::GET, api("/no-such-route"), Some(&admin), None).await;
    assert_eq!(st, 404, "{v}");
    assert_eq!(v["msg"], "接口不存在", "msg 不动: {v}");
    assert_eq!(v["err"], "common.not_found", "路由级 404 也要带码: {v}");

    let (st, v) = call(&c, Method::DELETE, api("/health"), None, None).await;
    assert_eq!(st, 405, "{v}");
    assert!(v.get("err").is_none(), "没有稳定码就整个不带 err 键（不是 null）: {v}");

    // ── 404 + 自定义提示：字典类型不存在（NotFoundMsg 也查表挂码）──
    let (st, v) = call(&c, Method::GET, api("/dicts/no_such_type/items"), Some(&admin), None).await;
    assert_eq!(st, 404, "{v}");
    assert_static_err(&v, "dict.type_missing");

    // ── 409 + args.code：库里残留的幽灵任务 ──
    let ghost = sqlx::query(
        "INSERT INTO job (name, code, cron, status, last_msg, created_at, updated_at) \
         VALUES ('幽灵任务', 'v17_ghost', '60', 1, '', NOW(), NOW())",
    )
    .execute(&pool)
    .await
    .unwrap()
    .last_insert_id();
    let (st, v) = call(&c, Method::POST, api(&format!("/jobs/{ghost}/run")), Some(&admin), None).await;
    assert_eq!(st, 409, "{v}");
    let args = assert_args_err(&v, "job.not_registered");
    assert_eq!(args["code"], "v17_ghost", "args.code 是任务的 code 原始值: {v}");

    // ── 429 + args.minutes：独立 IP 桶，不打扰别的用例 ──
    let login_url = api("/auth/login");
    for i in 1..=5 {
        let (st, v) = login_from(&c, &login_url, "7.7.7.7", "v17_lockme", "wrong").await;
        assert_eq!(st, 400, "第 {i} 次错密码还没到阈值: {v}");
        assert_static_err(&v, "auth.bad_credentials");
    }
    let (st, v) = login_from(&c, &login_url, "7.7.7.7", "v17_lockme", "wrong").await;
    assert_eq!(st, 429, "第 6 发吃限流: {v}");
    let args = assert_args_err(&v, "auth.throttled");
    assert_eq!(args["minutes"], 10, "lock_minutes 要作为原始值给前端拼「请 N 分钟后再试」: {v}");
    assert!(v["msg"].as_str().unwrap_or("").contains("频繁"), "msg 仍是中文原文: {v}");
}
