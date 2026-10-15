// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! CSV 导入（扩展工程项 3）：模板下载 + 上传导入的后端契约。
//!
//! 一个用例钉住施工图的四条验收：
//! 1. 正常样例全绿 —— 管理员 / 部门 / 字典项三种都导入成功，且**真能登录**（口令走了哈希路径）；
//! 2. 含坏行的样例**整批回滚** + 报告逐行指出（行号 + 原因），好行也没落库；
//! 3. 权限不越权 —— 没有 `:add` 码连模板都下不了；
//! 4. 数据范围不越权 —— 受限用户导入范围外的部门 / 更宽的角色，行内报错且零写入。
//! 另加审计留痕（写路径必须有）。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

const IMP: &str = "/api/v1/imports";
/// 所有测试账号共用的口令（≥8 位、字母+数字，满足复杂度下限）。
const PASS: &str = "t3pass123";

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

/// 上传一份 CSV（表单字段名与前端逐字一致：`file`）。
async fn post_csv(
    c: &reqwest::Client,
    url: String,
    token: Option<&str>,
    content: &str,
) -> (u16, Value) {
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(content.as_bytes().to_vec()).file_name("import.csv"),
    );
    let mut rb = c.post(url).multipart(form);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    let r = rb.send().await.expect("上传请求失败");
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

/// 下载模板：校验附件头 + BOM，返回去掉 BOM 的数据行。
async fn template(c: &reqwest::Client, url: String, token: &str) -> Vec<String> {
    let r = c.get(url).bearer_auth(token).send().await.expect("模板请求失败");
    assert_eq!(r.status(), 200, "模板必须 200");
    assert_eq!(r.headers()["content-type"], "text/csv; charset=utf-8", "模板 Content-Type");
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

/// 建部门（`parent = None` 建根），返回短串。
async fn make_dept(c: &reqwest::Client, base: &str, token: &str, name: &str) -> String {
    let (st, v) = call(
        c,
        Method::POST,
        format!("{base}/api/v1/depts"),
        Some(token),
        Some(json!({"parent_id": 0, "name": name, "sort": 1})),
    )
    .await;
    assert_eq!(st, 200, "建部门 {name}: {v}");
    common::as_id(&v["data"]["id"])
}

/// 建角色，返回短串。
async fn make_role(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    code: &str,
    data_scope: i32,
) -> String {
    let (st, v) = call(
        c,
        Method::POST,
        format!("{base}/api/v1/roles"),
        Some(token),
        Some(json!({"name": name, "code": code, "data_scope": data_scope, "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "建角色 {name}: {v}");
    common::as_id(&v["data"]["id"])
}

/// 报告里某行的失败原因（行号从 1 起；找不到行 = 断言失败）。
fn row_msg(report: &Value, line: u64) -> &str {
    let rows = report["data"]["rows"].as_array().expect("报告里要有 rows");
    rows.iter()
        .find(|r| r["line"].as_u64() == Some(line))
        .unwrap_or_else(|| panic!("报告里没有第 {line} 行: {report}"))["msg"]
        .as_str()
        .unwrap_or_default()
}

/// 报告里某行是否通过。
fn row_ok(report: &Value, line: u64) -> bool {
    let rows = report["data"]["rows"].as_array().expect("报告里要有 rows");
    rows.iter()
        .find(|r| r["line"].as_u64() == Some(line))
        .unwrap_or_else(|| panic!("报告里没有第 {line} 行: {report}"))["ok"]
        .as_bool()
        .unwrap_or(false)
}

#[tokio::test]
async fn csv_import_templates_validate_and_roll_back() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_srv, base) = common::start_server().await;
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");

    // ── 模板下载：BOM + 表头 + 只有表头（没有样例数据，免得手滑直接导入）──
    let c = reqwest::Client::new();
    let headers = [
        ("admin", "用户名,密码,昵称,邮箱,手机号,部门,角色,状态,备注"),
        ("dept", "部门名称,上级部门,负责人,联系电话,排序,状态"),
        ("dict-item", "类型编码,标签,值,排序,状态,备注"),
    ];
    for (kind, want) in headers {
        let lines = template(&c, format!("{base}{IMP}/template?kind={kind}"), &admin).await;
        assert_eq!(lines.len(), 1, "{kind} 模板只有表头: {lines:?}");
        assert_eq!(lines[0], want, "{kind} 模板表头");
    }
    let (st, v) = call(&c, Method::GET, format!("{base}{IMP}/template?kind=nope"), Some(&admin), None).await;
    assert_eq!(st, 400, "未知 kind 必须 400: {v}");

    // ── 部门导入：上级按名字解析（先建一个根部门当上级）──
    make_dept(&c, &base, &admin, "总部").await;
    let dept_csv = "\
部门名称,上级部门,负责人,联系电话,排序,状态
后端组,总部,王五,13900000001,10,1
";
    let (st, v) = post_csv(&c, format!("{base}{IMP}?kind=dept"), Some(&admin), dept_csv).await;
    assert_eq!(st, 200, "部门导入: {v}");
    assert_eq!(v["data"]["created"], 1, "部门导入 1 行: {v}");
    assert!(row_ok(&v, 2), "第 2 行该通过: {v}");

    // 树里确认：后端组挂在总部下
    let (_, tree) = call(&c, Method::GET, format!("{base}/api/v1/depts/tree"), Some(&admin), None).await;
    let zongbu = tree["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "总部")
        .unwrap_or_else(|| panic!("树里没有总部: {tree}"));
    let kids = zongbu["children"].as_array().cloned().unwrap_or_default();
    assert!(
        kids.iter().any(|d| d["name"] == "后端组"),
        "后端组该挂在总部下: {tree}"
    );

    // 上级不存在 → 行内报错 + 整批回滚
    let bad_dept = "\
部门名称,上级部门,负责人,联系电话,排序,状态
孤儿组,不存在的部门,赵六,,10,1
";
    let (st, v) = post_csv(&c, format!("{base}{IMP}?kind=dept"), Some(&admin), bad_dept).await;
    assert_eq!(st, 200, "坏行仍是 200 + 报告: {v}");
    assert_eq!(v["data"]["created"], 0, "坏行整批回滚: {v}");
    assert!(row_msg(&v, 2).contains("上级部门"), "第 2 行指出上级部门的问题: {v}");
    let (_, tree) = call(&c, Method::GET, format!("{base}/api/v1/depts/tree"), Some(&admin), None).await;
    assert!(
        !serde_json::to_string(&tree).unwrap().contains("孤儿组"),
        "坏行不落库: {tree}"
    );

    // ── 字典项导入：类型必须存在（先建类型）──
    let (st, v) = call(
        &c, Method::POST, format!("{base}/api/v1/dicts"), Some(&admin),
        Some(json!({"name": "导入色卡", "code": "imp_color", "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "建字典类型: {v}");
    let dict_csv = "\
类型编码,标签,值,排序,状态,备注
imp_color,红色,red,1,1,
imp_color,蓝色,blue,2,1,
";
    let (st, v) = post_csv(&c, format!("{base}{IMP}?kind=dict-item"), Some(&admin), dict_csv).await;
    assert_eq!(st, 200, "字典项导入: {v}");
    assert_eq!(v["data"]["created"], 2, "字典项导入 2 行: {v}");

    // 文件内值重复 → 第 3 行报错 + 整批回滚（第 2 行也不该落库）
    let dup_dict = "\
类型编码,标签,值,排序,状态,备注
imp_color,绿色,green,3,1,
imp_color,墨绿,green,4,1,
";
    let (st, v) = post_csv(&c, format!("{base}{IMP}?kind=dict-item"), Some(&admin), dup_dict).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["created"], 0, "重复值整批回滚: {v}");
    assert!(row_msg(&v, 3).contains("已存在"), "第 3 行指出值重复: {v}");
    let (_, items) = call(
        &c, Method::GET, format!("{base}/api/v1/dict-items?type_code=imp_color&size=100"), Some(&admin), None,
    )
    .await;
    assert_eq!(items["data"]["total"], 2, "回滚后仍是 2 条（绿色没落库）: {items}");

    // ── 管理员导入：正常样例全绿（含部门 + 两个角色）──
    make_role(&c, &base, &admin, "导入员", "imp_role", 1).await;
    make_role(&c, &base, &admin, "部门主管", "imp_lead", 3).await;
    let admin_csv = format!("\
用户名,密码,昵称,邮箱,手机号,部门,角色,状态,备注
imp_zhang,{PASS},张三,zhang@example.com,13800000001,后端组,导入员|部门主管,1,导入测试
imp_li,{PASS},李四,,,,,0,
");
    let (st, v) = post_csv(&c, format!("{base}{IMP}?kind=admin"), Some(&admin), &admin_csv).await;
    assert_eq!(st, 200, "管理员导入: {v}");
    assert_eq!(v["data"]["created"], 2, "管理员导入 2 行: {v}");
    assert_eq!(v["data"]["total"], 2, "总行数 2: {v}");
    assert!(row_ok(&v, 2) && row_ok(&v, 3), "两行都该通过: {v}");

    let (_, list) = call(
        &c, Method::GET, format!("{base}/api/v1/admins?username=imp_zhang"), Some(&admin), None,
    )
    .await;
    let row = &list["data"]["list"][0];
    assert_eq!(row["username"], "imp_zhang", "{list}");
    assert_eq!(row["dept_name"], "后端组", "部门按名字落对了: {list}");
    let mut roles: Vec<&str> = row["role_names"].as_array().unwrap().iter().filter_map(|r| r.as_str()).collect();
    roles.sort();
    assert_eq!(roles, vec!["导入员", "部门主管"], "两个角色都挂上: {list}");
    let (_, list2) = call(
        &c, Method::GET, format!("{base}/api/v1/admins?username=imp_li"), Some(&admin), None,
    )
    .await;
    assert_eq!(list2["data"]["list"][0]["status"], 0, "第 3 行状态 0 生效: {list2}");
    assert_eq!(list2["data"]["list"][0]["dept_name"], "", "空部门 = 无部门: {list2}");
    // 口令走的是真哈希：导入的账号能登录
    assert!(
        common::login(&base, "imp_zhang", PASS).await.is_some(),
        "导入的账号必须能用导入的口令登录"
    );

    // ── 坏样例：好行 + 弱口令 + 用户名重复 → 整批回滚，好行也不落库 ──
    let bad_csv = format!("\
用户名,密码,昵称,邮箱,手机号,部门,角色,状态,备注
imp_ok,{PASS},好行,,,,,1,
imp_weak,123,弱口令行,,,,,1,
imp_ok,{PASS},重名行,,,,,1,
");
    let (st, v) = post_csv(&c, format!("{base}{IMP}?kind=admin"), Some(&admin), &bad_csv).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["created"], 0, "有坏行整批回滚: {v}");
    assert!(row_ok(&v, 2), "第 2 行本身没问题: {v}");
    assert!(row_msg(&v, 3).contains("至少 8 位"), "第 3 行指出口令弱: {v}");
    assert!(row_msg(&v, 4).contains("已存在"), "第 4 行指出用户名重复: {v}");
    let (_, list) = call(
        &c, Method::GET, format!("{base}/api/v1/admins?username=imp_ok"), Some(&admin), None,
    )
    .await;
    assert_eq!(list["data"]["total"], 0, "好行也必须回滚: {list}");

    // ── 权限：只有 list 码的用户连模板都下不了，导入同样 403 ──
    let (_, tree) = call(&c, Method::GET, format!("{base}/api/v1/menus/tree"), Some(&admin), None).await;
    let list_menu = menu_id(&tree["data"], "system:admin:list").expect("种子里有管理员列表码");
    let (_, none_role) = call(
        &c, Method::POST, format!("{base}/api/v1/roles"), Some(&admin),
        Some(json!({"name": "导入只读", "code": "imp_viewer", "data_scope": 5, "status": 1})),
    )
    .await;
    let none_role = common::as_id(&none_role["data"]["id"]);
    let (st, v) = call(
        &c, Method::PUT, format!("{base}/api/v1/roles/{none_role}/menus"), Some(&admin),
        Some(json!({"menu_ids": [list_menu]})),
    )
    .await;
    assert_eq!(st, 200, "挂 list 菜单: {v}");
    let (st, v) = call(
        &c, Method::POST, format!("{base}/api/v1/admins"), Some(&admin),
        Some(json!({"username": "impv", "password": "impv12345", "role_ids": [none_role]})),
    )
    .await;
    assert_eq!(st, 200, "建受限用户: {v}");
    let viewer = common::login(&base, "impv", "impv12345").await.expect("受限用户登录");
    let (st, v) = call(&c, Method::GET, format!("{base}{IMP}/template?kind=admin"), Some(&viewer), None).await;
    assert_eq!(st, 403, "没有 add 码连模板都下不了: {v}");
    let (st, v) = post_csv(&c, format!("{base}{IMP}?kind=admin"), Some(&viewer), &admin_csv).await;
    assert_eq!(st, 403, "没有 add 码不能导入: {v}");

    // ── 数据范围：自定义范围（总部）的用户，导不了范围外的部门 / 更宽的角色 ──
    make_dept(&c, &base, &admin, "集团").await; // 范围外的另一个部门
    let scope_role = make_role(&c, &base, &admin, "总部专员", "imp_hq", 5).await;
    let (st, v) = call(
        &c, Method::PUT, format!("{base}/api/v1/roles/{scope_role}/depts"), Some(&admin),
        Some(json!({"dept_ids": [zongbu["id"].clone()]})),
    )
    .await;
    assert_eq!(st, 200, "给角色圈定部门: {v}");
    let add_menu = menu_id(&tree["data"], "system:admin:add").expect("种子里有管理员新增码");
    let (st, v) = call(
        &c, Method::PUT, format!("{base}/api/v1/roles/{scope_role}/menus"), Some(&admin),
        Some(json!({"menu_ids": [list_menu, add_menu]})),
    )
    .await;
    assert_eq!(st, 200, "挂 list+add 菜单: {v}");
    let (st, v) = call(
        &c, Method::POST, format!("{base}/api/v1/admins"), Some(&admin),
        Some(json!({"username": "imphq", "password": "imphq12345", "dept_id": zongbu["id"].clone(), "role_ids": [scope_role]})),
    )
    .await;
    assert_eq!(st, 200, "建总部专员: {v}");
    let hq = common::login(&base, "imphq", "imphq12345").await.expect("总部专员登录");

    // (a) 范围内：导得进去（没被一刀切挡掉）
    let in_scope = format!("\
用户名,密码,昵称,邮箱,手机号,部门,角色,状态,备注
imp_in,{PASS},范围内,,,总部,,1,
");
    let (st, v) = post_csv(&c, format!("{base}{IMP}?kind=admin"), Some(&hq), &in_scope).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["created"], 1, "范围内的行该导入成功: {v}");

    // (b) 范围外部门 + 更宽角色（data_scope=1：「更宽」的判定与同步接口同一份规则）
    let out_scope = format!("\
用户名,密码,昵称,邮箱,手机号,部门,角色,状态,备注
imp_out1,{PASS},范围外部门,,,集团,,1,
imp_out2,{PASS},更宽角色,,,总部,导入员,1,
");
    let (st, v) = post_csv(&c, format!("{base}{IMP}?kind=admin"), Some(&hq), &out_scope).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["created"], 0, "越权行整批回滚: {v}");
    assert!(row_msg(&v, 2).contains("数据权限"), "第 2 行指出部门越界: {v}");
    assert!(row_msg(&v, 3).contains("更宽"), "第 3 行指出角色越界: {v}");
    let (_, list) = call(
        &c, Method::GET, format!("{base}/api/v1/admins?username=imp_out"), Some(&admin), None,
    )
    .await;
    assert_eq!(list["data"]["total"], 0, "越权行不落库: {list}");

    // ── 写路径留痕：审计里有 import 模块的记录 ──
    let (st, v) = call(
        &c, Method::GET, format!("{base}/api/v1/audit-logs?module=import&size=100"), Some(&admin), None,
    )
    .await;
    assert_eq!(st, 200, "审计列表: {v}");
    let rows = v["data"]["list"].as_array().expect("审计 list");
    assert!(!rows.is_empty(), "导入要有审计留痕: {v}");
    let actions: Vec<&str> = rows.iter().filter_map(|r| r["action"].as_str()).collect();
    assert!(actions.iter().any(|a| a.contains("导入")), "动作名: {actions:?}");
}
