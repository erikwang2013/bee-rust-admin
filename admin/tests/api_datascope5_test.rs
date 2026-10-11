// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 数据权限 5 档（自定义）集成测试。5 档是 `role_dept` 里勾中的部门**扁平集合**，
//! 不是子树 —— 勾了「B部」不会带出「B1组」，这正是它与 2 档最大的区别。
//! 钉四件事：
//! 1. 可以跨分支勾选（看别人的部门、看不到自己所在的部门）；
//! 2. 只勾父不勾子 → 子部门的看不见（排除「被实现成 2 档」）；
//! 3. 一个部门都没勾 → 什么都看不到（含自己），是 200 空列表而非 403；
//! 4. 多角色取并集，任一角色 1 档 → 全部；筛选参数只能收窄、不能放大。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

/// 所有测试账号共用的密码（≥8 位、字母+数字，满足复杂度下限）。
const PASS: &str = "t5pass123";

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

/// 设置 5 档角色的自定义部门范围，并回读确认落库（顺序无关，别依赖返回顺序）。
async fn set_role_depts(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    role: &str,
    depts: &[String],
) {
    let (st, v) = call(
        c,
        Method::PUT,
        format!("{base}/api/v1/roles/{role}/depts"),
        Some(token),
        Some(json!({"dept_ids": depts})),
    )
    .await;
    assert_eq!(st, 200, "设置角色部门: {v}");
    let (st, v) =
        call(c, Method::GET, format!("{base}/api/v1/roles/{role}/depts"), Some(token), None).await;
    assert_eq!(st, 200, "回读角色部门: {v}");
    let mut got: Vec<String> = v["data"]
        .as_array()
        .unwrap_or_else(|| panic!("data 应是数组: {v}"))
        .iter()
        .map(common::as_id)
        .collect();
    got.sort_unstable();
    let mut want = depts.to_vec();
    want.sort_unstable();
    assert_eq!(got, want, "部门范围必须落库: {v}");
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
    assert_eq!(st, 200, "列表应 200（范围为空是空列表，不是 403）: {v}");
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

#[tokio::test]
async fn datascope5_custom_depts_visibility() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");

    let admin_token = common::login(&base, "admin", "admin123").await.expect("超管登录失败");

    // ── 部门树：总部 ├ A部（└ A1组）└ B部（└ B1组）
    let hq = make_dept(&c, &base, &admin_token, "总部", None).await;
    let a = make_dept(&c, &base, &admin_token, "A部", Some(&hq)).await;
    let a1 = make_dept(&c, &base, &admin_token, "A1组", Some(&a)).await;
    let b = make_dept(&c, &base, &admin_token, "B部", Some(&hq)).await;
    let b1 = make_dept(&c, &base, &admin_token, "B1组", Some(&b)).await;

    // ── 5 个角色（造数据全程超管，不受授予校验限制）
    let (st, menus) = call(&c, Method::GET, api("/menus/tree"), Some(&admin_token), None).await;
    assert_eq!(st, 200, "菜单树: {menus}");
    let admin_menu =
        menu_id(&menus["data"], "system:admin:list").expect("种子菜单含 system:admin:list");
    let role_menus = vec![admin_menu];
    let cross = make_role(&c, &base, &admin_token, "t5_cross", 5, &role_menus).await;
    let bonly = make_role(&c, &base, &admin_token, "t5_bonly", 5, &role_menus).await;
    let no_dept = make_role(&c, &base, &admin_token, "t5_none", 5, &role_menus).await;
    let dept3 = make_role(&c, &base, &admin_token, "t5_dept3", 3, &role_menus).await;
    let scope_all = make_role(&c, &base, &admin_token, "t5_all", 1, &role_menus).await;

    // ── 5 档的部门范围：跨分支勾 / 只勾父 / 一个都不勾（后者是空数组，不是不调）
    set_role_depts(&c, &base, &admin_token, &cross, &[b.clone(), b1.clone()]).await;
    set_role_depts(&c, &base, &admin_token, &bonly, &[b.clone()]).await;
    set_role_depts(&c, &base, &admin_token, &no_dept, &[]).await;
    // t5_dept3 / t5_all 不调 depts 接口：它们不该靠勾部门工作

    // ── 10 个管理员（admin 由 seed 建，无部门）
    make_admin(&c, &base, &admin_token, "t5_op", Some(&a), &[cross.clone()]).await;
    make_admin(&c, &base, &admin_token, "t5_b", Some(&b), &[cross.clone()]).await;
    make_admin(&c, &base, &admin_token, "t5_b1", Some(&b1), &[cross.clone()]).await;
    make_admin(&c, &base, &admin_token, "t5_a", Some(&a), &[cross.clone()]).await;
    make_admin(&c, &base, &admin_token, "t5_a1", Some(&a1), &[cross.clone()]).await;
    make_admin(&c, &base, &admin_token, "t5_bonly_op", Some(&a), &[bonly.clone()]).await;
    make_admin(&c, &base, &admin_token, "t5_none_op", Some(&b), &[no_dept.clone()]).await;
    make_admin(&c, &base, &admin_token, "t5_multi", Some(&a), &[dept3, cross.clone()]).await;
    make_admin(&c, &base, &admin_token, "t5_all_op", Some(&a1), &[cross.clone(), scope_all]).await;

    // ── 5 个操作者登录
    let op = common::login(&base, "t5_op", PASS).await.expect("t5_op 登录失败");
    let bonly_op = common::login(&base, "t5_bonly_op", PASS).await.expect("t5_bonly_op 登录失败");
    let none_op = common::login(&base, "t5_none_op", PASS).await.expect("t5_none_op 登录失败");
    let multi = common::login(&base, "t5_multi", PASS).await.expect("t5_multi 登录失败");
    let all_op = common::login(&base, "t5_all_op", PASS).await.expect("t5_all_op 登录失败");

    // ── B1 跨分支勾选：看得见 B 支，看不见自己所在的 A 支
    let (names, total) = list_names(&c, api("/admins?size=100"), &op).await;
    assert_eq!(total, 3, "勾了 B部+B1组 应见 3 人: {names:?}");
    assert_eq!(
        names,
        vec!["t5_b", "t5_b1", "t5_none_op"],
        "跨分支可见、自己的分支不可见: {names:?}"
    );

    // ── B2 扁平集合不是子树：只勾 B部 → B1组 的人看不见
    //    （与 B1 成对：同一个 A部 的人，勾了 B1组 就能看到它、只勾 B部 就看不到）
    let (names, total) = list_names(&c, api("/admins?size=100"), &bonly_op).await;
    assert_eq!(total, 2, "只勾 B部 应见 2 人（不含 B1组 的）：{names:?}");
    assert_eq!(names, vec!["t5_b", "t5_none_op"], "5 档是扁平集合，不带出子部门: {names:?}");

    // ── B3 一个部门都没勾：含自己在内一条都看不到，且是 200 空列表
    let (st, v) = call(&c, Method::GET, api("/admins?size=100"), Some(&none_op), None).await;
    assert_eq!(st, 200, "没勾部门是空列表，不是 403: {v}");
    assert_eq!(v["data"]["total"], 0, "没勾部门 = 什么都看不到（含自己）: {v}");
    assert_eq!(v["data"]["list"], json!([]), "list 必须是空数组: {v}");

    // ── B4 多角色取并集：3 档（A部）∪ 5 档（B部+B1组）＝ A部 ∪ B支；
    //    自己是 A部 所以自己也在里面（3 档带出来的），A1组 与超管仍不可见
    let (names, total) = list_names(&c, api("/admins?size=100"), &multi).await;
    assert_eq!(total, 7, "多角色应取并集: {names:?}");
    assert_eq!(
        names,
        vec!["t5_a", "t5_b", "t5_b1", "t5_bonly_op", "t5_multi", "t5_none_op", "t5_op"],
        "并集 = A部 ∪ B部 ∪ B1组，不含 A1组 与超管: {names:?}"
    );

    // ── B5 任一角色 1 档 → 全部（含超管）
    let (names, total) = list_names(&c, api("/admins?size=100"), &all_op).await;
    assert_eq!(total, 10, "1 档应见全部 10 人: {names:?}");
    assert_eq!(
        names,
        vec![
            "admin",
            "t5_a",
            "t5_a1",
            "t5_all_op",
            "t5_b",
            "t5_b1",
            "t5_bonly_op",
            "t5_multi",
            "t5_none_op",
            "t5_op"
        ],
        "任一 1 档角色 → 全部: {names:?}"
    );

    // ── B6/B7 筛选参数与数据范围是「与」：能收窄，不能放大
    let (st, v) =
        call(&c, Method::GET, api(&format!("/admins?size=100&dept_id={a}")), Some(&op), None).await;
    assert_eq!(st, 200, "按 A部 筛选: {v}");
    assert_eq!(v["data"]["total"], 0, "筛自己部门的别想看到 A部 的人: {v}");
    let (names, total) = list_names(&c, api(&format!("/admins?size=100&dept_id={b}")), &op).await;
    assert_eq!(total, 2, "范围内按 B部 精确筛选: {names:?}");
    assert_eq!(
        names,
        vec!["t5_b", "t5_none_op"],
        "筛 B部 只留直属该部门的两人（B1组 不在内）: {names:?}"
    );
}
