// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 数据权限 2 档（本部门及以下）集成测试。一个用例钉三件事：
//! 1. 2 档 = 自己部门的**整棵子树**，列表 / 登录记录（`admin_id` 子查询）/ 导出 / 详情
//!    四条路径同口径；
//! 2. 无部门者（`dept_id = 0`）`subtree(0)` 为空 → 什么都看不到，且是 200 空列表而非 403；
//! 3. 超管 dept 也是 0 → 不在任何人的子树里，2 档永远看不到超管。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

/// 所有测试账号共用的密码（≥6 位）。
const PASS: &str = "t2pass123";

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
async fn datascope2_subtree_visibility() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");

    let admin_token = common::login(&base, "admin", "admin123").await.expect("超管登录失败");

    // ── 部门树：总部 ├ 研发部（├ 前端组 ├ 后端组）└ 市场部（└ 华东区）
    let hq = make_dept(&c, &base, &admin_token, "总部", None).await;
    let rd = make_dept(&c, &base, &admin_token, "研发部", Some(&hq)).await;
    let fe = make_dept(&c, &base, &admin_token, "前端组", Some(&rd)).await;
    let be = make_dept(&c, &base, &admin_token, "后端组", Some(&rd)).await;
    let mkt = make_dept(&c, &base, &admin_token, "市场部", Some(&hq)).await;
    let east = make_dept(&c, &base, &admin_token, "华东区", Some(&mkt)).await;

    // ── 角色：2 档 + 管理员列表 / 登录记录两个 list 权限（造数据全程超管，不受授予校验限制）
    let (st, menus) = call(&c, Method::GET, api("/menus/tree"), Some(&admin_token), None).await;
    assert_eq!(st, 200, "菜单树: {menus}");
    let admin_menu =
        menu_id(&menus["data"], "system:admin:list").expect("种子菜单含 system:admin:list");
    let log_menu =
        menu_id(&menus["data"], "system:loginlog:list").expect("种子菜单含 system:loginlog:list");
    let role = make_role(&c, &base, &admin_token, "t2_role", 2, &[admin_menu, log_menu]).await;
    let roles = vec![role.clone()];

    // ── 8 个管理员（admin 由 seed 建，dept=0）；只有 A4/A5 要 t2_east / t2_fe 的 id
    let _ = make_admin(&c, &base, &admin_token, "t2_rd", Some(&rd), &roles).await;
    let t2_fe = make_admin(&c, &base, &admin_token, "t2_fe", Some(&fe), &roles).await;
    make_admin(&c, &base, &admin_token, "t2_be", Some(&be), &roles).await;
    make_admin(&c, &base, &admin_token, "t2_mkt", Some(&mkt), &roles).await;
    let t2_east = make_admin(&c, &base, &admin_token, "t2_east", Some(&east), &roles).await;
    make_admin(&c, &base, &admin_token, "t2_root", Some(&hq), &roles).await;
    make_admin(&c, &base, &admin_token, "t2_none", None, &roles).await;

    // ── 8 个用户各登录一次（登录记录靠它产生），token 只有 3 个操作者要留
    let t2_rd_token = common::login(&base, "t2_rd", PASS).await.expect("t2_rd 登录失败");
    let t2_none_token = common::login(&base, "t2_none", PASS).await.expect("t2_none 登录失败");
    let t2_root_token = common::login(&base, "t2_root", PASS).await.expect("t2_root 登录失败");
    for u in ["t2_fe", "t2_be", "t2_mkt", "t2_east"] {
        common::login(&base, u, PASS).await.unwrap_or_else(|| panic!("{u} 登录失败"));
    }

    // ── A1 列表：t2_rd（研发部）只见研发部子树 3 人；市场部整支与超管/无部门者都不见
    let (names, total) = list_names(&c, api("/admins?size=100"), &t2_rd_token).await;
    assert_eq!(total, 3, "2 档应见自己子树 3 人（研发部 + 两个组）: {names:?}");
    assert_eq!(names, vec!["t2_be", "t2_fe", "t2_rd"], "2 档应只见自己子树: {names:?}");

    // ── A2 登录记录走 admin_id 子查询，同口径
    let (names, total) = list_names(&c, api("/login-logs?size=100"), &t2_rd_token).await;
    assert_eq!(total, 3, "登录记录应只含子树的 3 人: {names:?}");
    assert_eq!(names, vec!["t2_be", "t2_fe", "t2_rd"], "子查询路径同口径: {names:?}");

    // ── A3 导出不能绕过数据权限（集合精确相等，天然排除 t2_mkt / t2_east / t2_none / admin）
    let lines = csv_lines(&c, api("/admins/export"), &t2_rd_token).await;
    assert_eq!(csv_usernames(&lines), vec!["t2_be", "t2_fe", "t2_rd"], "导出不能绕过: {lines:?}");

    // ── A4/A5 详情与列表同口径：越界 403，界内 200
    let (st, v) =
        call(&c, Method::GET, api(&format!("/admins/{t2_east}")), Some(&t2_rd_token), None).await;
    assert_eq!(st, 403, "兄弟分支子部门的详情必须 403: {v}");
    assert!(v["msg"].as_str().unwrap_or("").contains("超出你的数据权限范围"), "403 文案: {v}");
    let (st, v) =
        call(&c, Method::GET, api(&format!("/admins/{t2_fe}")), Some(&t2_rd_token), None).await;
    assert_eq!(st, 200, "子树内的详情必须 200: {v}");

    // ── A6 根部门操作者看整片森林，但仍看不到超管与无部门者（dept 0 不在任何子树里）
    let (names, total) = list_names(&c, api("/admins?size=100"), &t2_root_token).await;
    assert_eq!(total, 6, "总部子树 = 6 人（不含超管与无部门者）: {names:?}");
    assert_eq!(
        names,
        vec!["t2_be", "t2_east", "t2_fe", "t2_mkt", "t2_rd", "t2_root"],
        "整棵树可见，超管/无部门者不可见: {names:?}"
    );

    // ── A7/A8/A9 无部门者（dept_id = 0）：subtree(0) 为空 → 什么都看不到，
    //    且是 200 空列表（不是 403，也不是「看到全公司」）
    let (st, v) = call(&c, Method::GET, api("/admins?size=100"), Some(&t2_none_token), None).await;
    assert_eq!(st, 200, "无部门者看列表是空列表，不是 403: {v}");
    assert_eq!(v["data"]["total"], 0, "无部门者一条都看不到: {v}");
    assert_eq!(v["data"]["list"], json!([]), "list 必须是空数组: {v}");
    let (names, total) = list_names(&c, api("/login-logs?size=100"), &t2_none_token).await;
    assert_eq!(total, 0, "无部门者的登录记录也是空: {names:?}");
    let lines = csv_lines(&c, api("/admins/export"), &t2_none_token).await;
    assert_eq!(lines.len() - 1, 0, "无部门者导出只有表头: {lines:?}");
    assert!(csv_usernames(&lines).is_empty(), "无部门者导出没有数据行: {lines:?}");
}
