// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 数据权限 4 档（仅本人）集成测试。4 档**不碰 `dept_ids`**，只把 `self_only` 置真，
//! 于是条件生成（`DataScope::condition` / `admin_id_condition`）退化成
//! `(id = 自己)` / `(admin_id = 自己)`。一个用例钉四件事：
//! 1. 列表 / 登录记录 / 导出 / 详情四条路径同口径 —— 只看得到自己**一个人**；
//! 2. 同部门同事不可见：4 档与 3 档（本部门）的分水岭；
//! 3. 子部门同事不可见：4 档与 2 档（本部门及以下）的分水岭；
//! 4. 自己可读（别把闸门写死成一律 403），且 `dept_id` 筛选只能收窄——自己本来就在 A部。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

/// 所有测试账号共用的密码（≥8 位、字母+数字，满足复杂度下限）。
const PASS: &str = "t4pass123";

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

/// 取 CSV 响应体（含 BOM），校验附件头，返回去掉 BOM 的文本行（照抄 api_v14_test.rs）。
async fn csv_lines(c: &reqwest::Client, url: String, token: &str) -> Vec<String> {
    let r = c.get(url).bearer_auth(token).send().await.expect("导出请求失败");
    assert_eq!(r.status(), 200, "导出必须 200");
    assert_eq!(r.headers()["content-type"], "text/csv; charset=utf-8", "导出 Content-Type");
    let cd = r.headers()["content-disposition"].to_str().unwrap().to_string();
    assert!(cd.contains("attachment") && cd.contains(".csv"), "附件头: {cd}");
    let body = r.bytes().await.unwrap().to_vec();
    assert_eq!(&body[..3], &[0xEF, 0xBB, 0xBF], "首字节必须是 UTF-8 BOM");
    String::from_utf8(body[3..].to_vec())
        .expect("BOM 之后是 UTF-8")
        .trim_end()
        .split("\r\n")
        .map(str::to_string)
        .collect()
}

/// 菜单树里按权限码取菜单 id（按码找，不按名字）。
fn menu_id(menus: &Value, perm: &str) -> Option<String> {
    for m in menus.as_array()? {
        if m["perm"].as_str() == Some(perm) {
            return m["id"].as_str().map(str::to_string);
        }
        if let Some(id) = menu_id(&m["children"], perm) {
            return Some(id);
        }
    }
    None
}

/// 建部门（`parent = None` 建根，传 0 哨兵），返回短串。
async fn make_dept(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    parent: Option<&str>,
) -> String {
    let pid = match parent {
        Some(h) => json!(h),
        None => json!(0),
    };
    let (st, v) = call(
        c,
        Method::POST,
        format!("{base}/api/v1/depts"),
        Some(token),
        Some(json!({"parent_id": pid, "name": name, "sort": 1})),
    )
    .await;
    assert_eq!(st, 200, "建部门 {name}: {v}");
    common::as_id(&v["data"]["id"])
}

/// 建角色 + 勾菜单，返回角色短串。
async fn make_role(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    scope: i8,
    menu_ids: &[String],
) -> String {
    let (st, v) = call(
        c,
        Method::POST,
        format!("{base}/api/v1/roles"),
        Some(token),
        Some(json!({"name": name, "code": name, "data_scope": scope, "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "建角色 {name}: {v}");
    let id = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        c,
        Method::PUT,
        format!("{base}/api/v1/roles/{id}/menus"),
        Some(token),
        Some(json!({"menu_ids": menu_ids})),
    )
    .await;
    assert_eq!(st, 200, "勾菜单 {name}: {v}");
    id
}

/// 建管理员（`dept = None` = 无部门），返回短串。
async fn make_admin(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    username: &str,
    dept: Option<&str>,
    role_ids: &[String],
) -> String {
    let dept = match dept {
        Some(h) => json!(h),
        None => json!(0),
    };
    let (st, v) = call(
        c,
        Method::POST,
        format!("{base}/api/v1/admins"),
        Some(token),
        Some(json!({
            "username": username, "password": PASS, "nickname": username,
            "dept_id": dept, "role_ids": role_ids,
        })),
    )
    .await;
    assert_eq!(st, 200, "建管理员 {username}: {v}");
    common::as_id(&v["data"]["id"])
}

/// 打列表接口：断言 200，返回 (排序后的用户名集合, total)。
/// 只断言**集合**，不依赖 `id DESC` 的返回顺序。
async fn list_names(c: &reqwest::Client, url: String, token: &str) -> (Vec<String>, i64) {
    let (st, v) = call(c, Method::GET, url, Some(token), None).await;
    assert_eq!(st, 200, "列表应 200（数据权限为空是空列表，不是 403）: {v}");
    assert!(v["data"]["list"].is_array(), "list 必须是数组: {v}");
    let mut names: Vec<String> = v["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r["username"].as_str().unwrap_or_else(|| panic!("username 不是字符串: {v}")).to_string()
        })
        .collect();
    names.sort_unstable();
    let total = v["data"]["total"].as_i64().unwrap_or_else(|| panic!("total 不是整数: {v}"));
    assert_eq!(names.len() as i64, total, "size=100 时 list 长度应等于 total: {v}");
    (names, total)
}

/// CSV 数据行（跳过表头）的用户名列。列序见 `admin.rs::export_header`，
/// 用户名固定在第 2 列；这里的字段都不含逗号，直接按 `,` 切即可。
fn csv_usernames(lines: &[String]) -> Vec<String> {
    let mut names: Vec<String> = lines
        .iter()
        .skip(1)
        .map(|l| {
            l.split(',').nth(1).unwrap_or_else(|| panic!("CSV 行没有第 2 列: {l}")).to_string()
        })
        .collect();
    names.sort_unstable();
    names
}

#[tokio::test]
async fn datascope4_self_only_visibility() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");

    let admin_token = common::login(&base, "admin", "admin123").await.expect("超管登录失败");

    // ── 部门树：总部 └ A部 └ A1组（t4_op 与 t4_peer 同在 A部，t4_child 在子部门 A1组）
    let hq = make_dept(&c, &base, &admin_token, "总部", None).await;
    let a = make_dept(&c, &base, &admin_token, "A部", Some(&hq)).await;
    let a1 = make_dept(&c, &base, &admin_token, "A1组", Some(&a)).await;

    // ── 角色：4 档 + 管理员列表 / 登录记录两个 list 权限（造数据全程超管，不受授予校验限制）
    let (st, menus) = call(&c, Method::GET, api("/menus/tree"), Some(&admin_token), None).await;
    assert_eq!(st, 200, "菜单树: {menus}");
    let admin_menu =
        menu_id(&menus["data"], "system:admin:list").expect("种子菜单含 system:admin:list");
    let log_menu =
        menu_id(&menus["data"], "system:loginlog:list").expect("种子菜单含 system:loginlog:list");
    let role = make_role(&c, &base, &admin_token, "t4_self", 4, &[admin_menu, log_menu]).await;

    // ── 3 个管理员（admin 由 seed 建，dept=0，天然不可见）；t4_peer / t4_child 无角色，只当见证人
    let t4_op = make_admin(&c, &base, &admin_token, "t4_op", Some(&a), &[role.clone()]).await;
    let t4_peer = make_admin(&c, &base, &admin_token, "t4_peer", Some(&a), &[]).await;
    make_admin(&c, &base, &admin_token, "t4_child", Some(&a1), &[]).await;

    // ── 三个人各登录一次（登录记录靠它产生），只有 t4_op 的 token 要用
    let t4_op_token = common::login(&base, "t4_op", PASS).await.expect("t4_op 登录失败");
    for u in ["t4_peer", "t4_child"] {
        common::login(&base, u, PASS).await.unwrap_or_else(|| panic!("{u} 登录失败"));
    }

    // ── A1 列表：只看得到自己 —— 同部门的 t4_peer、子部门的 t4_child、超管都不可见
    let (names, total) = list_names(&c, api("/admins?size=100"), &t4_op_token).await;
    assert_eq!(total, 1, "4 档应只见自己 1 人（同部门也不行）: {names:?}");
    assert_eq!(names, vec!["t4_op"], "4 档的列表集合恰为自己: {names:?}");

    // ── A2 登录记录走 admin_id 条件，同口径：同/子部门的登录记录都不可见
    let (names, total) = list_names(&c, api("/login-logs?size=100"), &t4_op_token).await;
    assert_eq!(total, 1, "登录记录应只含自己 1 条: {names:?}");
    assert_eq!(names, vec!["t4_op"], "登录记录集合恰为自己: {names:?}");

    // ── A3 导出不能绕过数据权限（集合精确相等，天然排除 t4_peer / t4_child / admin）
    let lines = csv_lines(&c, api("/admins/export"), &t4_op_token).await;
    assert_eq!(lines.len() - 1, 1, "导出应恰有 1 条数据行: {lines:?}");
    assert_eq!(csv_usernames(&lines), vec!["t4_op"], "导出不能绕过: {lines:?}");

    // ── A4/A5 详情与列表同口径：同部门同事也 403，自己 200
    let (st, v) = call(&c, Method::GET, api(&format!("/admins/{t4_peer}")), Some(&t4_op_token), None).await;
    assert_eq!(st, 403, "同部门同事的详情必须 403（4 档 ≠ 3 档）: {v}");
    assert!(v["msg"].as_str().unwrap_or("").contains("超出你的数据权限范围"), "403 文案: {v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/admins/{t4_op}")), Some(&t4_op_token), None).await;
    assert_eq!(st, 200, "自己的详情必须 200: {v}");

    // ── A6 筛选只能收窄：A部 里本来有 2 人（t4_op + t4_peer），带上 dept_id 仍只有自己
    let (names, total) =
        list_names(&c, api(&format!("/admins?size=100&dept_id={a}")), &t4_op_token).await;
    assert_eq!(total, 1, "dept_id 筛选不能放宽数据权限: {names:?}");
    assert_eq!(names, vec!["t4_op"], "筛选后仍恰为自己: {names:?}");
}
