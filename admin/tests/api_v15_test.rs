// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! BRD v1.5 后端新能力集成测试：C1 字典管理。
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

/// 取 CSV 响应体（含 BOM），校验附件头，返回去掉 BOM 的文本行。
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

/// 菜单树里按权限码找菜单节点。
fn menu<'a>(menus: &'a Value, perm: &str) -> Option<&'a Value> {
    for m in menus.as_array().into_iter().flatten() {
        if m["perm"].as_str() == Some(perm) {
            return Some(m);
        }
        if let Some(hit) = menu(&m["children"], perm) {
            return Some(hit);
        }
    }
    None
}

#[tokio::test]
async fn v15_dict_features() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;

    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");

    // ── 种子菜单：字典管理挂在系统管理下，按钮码齐（侧边栏从库里来）──
    let (st, v) = call(&c, Method::GET, api("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200, "菜单树: {v}");
    let tree = v["data"].clone();
    let dict_menu = menu(&tree, "system:dict:list")
        .unwrap_or_else(|| panic!("种子里必须有字典管理菜单: {tree}"));
    assert_eq!(dict_menu["path"], "/system/dict", "页面路径");
    assert_eq!(dict_menu["component"], "system/dict/index", "组件路径");
    let dict_id = dict_menu["id"].as_u64().unwrap();
    let parent_id = dict_menu["parent_id"].as_u64().unwrap();
    let system_dir = tree
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == json!(parent_id))
        .unwrap_or_else(|| panic!("字典菜单的父节点必须是系统管理: {tree}"));
    assert_eq!(system_dir["path"], "/system", "挂在系统管理下: {system_dir}");
    for btn in ["add", "edit", "remove"] {
        let perm = format!("system:dict:{btn}");
        let m = menu(&tree, &perm).unwrap_or_else(|| panic!("缺按钮权限码 {perm}: {tree}"));
        assert_eq!(m["parent_id"], json!(dict_id), "{perm} 挂在字典菜单下");
    }

    // ── 字典类型 CRUD ──────────────────────────────────────
    let (st, v) = call(&c, Method::GET, api("/dicts"), Some(&admin), None).await;
    assert_eq!(st, 200, "空列表: {v}");
    assert_eq!(v["data"]["total"], 0, "{v}");

    let (st, v) = call(&c, Method::POST, api("/dicts"), Some(&admin), Some(json!({
        "name": "用户性别", "code": "user_sex", "status": 1, "remark": "性别枚举",
    }))).await;
    assert_eq!(st, 200, "建类型: {v}");
    let sex_id = v["data"]["id"].as_u64().expect("创建要回 id");

    let (st, v) = call(&c, Method::POST, api("/dicts"), Some(&admin), Some(json!({
        "name": "订单状态", "code": "order_status", "status": 0, "remark": "",
    }))).await;
    assert_eq!(st, 200, "建第二个类型: {v}");
    let order_id = v["data"]["id"].as_u64().unwrap();

    // 下拉的两种“空”要分开：类型不存在 → 404；类型在但没配条目 → 200 + []
    let (st, v) = call(&c, Method::GET, api("/dicts/no_such_code/items"), Some(&admin), None).await;
    assert_eq!(st, 404, "拼错 code 必须 404，不能是「空下拉」: {v}");
    assert_eq!(v["msg"], "字典类型不存在", "{v}");
    assert_eq!(v["err"], "dict.type_missing", "稳定码要能对上表: {v}");
    assert!(v["data"].is_null(), "错误信封形状: {v}");
    let (st, v) = call(&c, Method::GET, api("/dicts/order_status/items"), Some(&admin), None).await;
    assert_eq!(st, 200, "类型在、没配条目是合法状态，不是错误: {v}");
    assert_eq!(v["data"], json!([]), "{v}");

    // 列表 + 筛选
    let (st, v) = call(&c, Method::GET, api("/dicts?page=1&size=10"), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 2, "{v}");
    let (_st, v) = call(&c, Method::GET, api("/dicts?name=性别"), Some(&admin), None).await;
    assert_eq!(v["data"]["total"], 1, "name 模糊筛选: {v}");
    assert_eq!(v["data"]["list"][0]["code"], "user_sex", "{v}");
    let (_st, v) = call(&c, Method::GET, api("/dicts?name=不存在的"), Some(&admin), None).await;
    assert_eq!(v["data"]["total"], 0, "{v}");
    let (_st, v) = call(&c, Method::GET, api("/dicts?status=0"), Some(&admin), None).await;
    assert_eq!(v["data"]["total"], 1, "status 筛选: {v}");
    assert_eq!(v["data"]["list"][0]["code"], "order_status", "{v}");

    // 重复 code → 400（友好提示，不是 500）
    let (st, v) = call(&c, Method::POST, api("/dicts"), Some(&admin), Some(json!({
        "name": "重复", "code": "user_sex", "status": 1,
    }))).await;
    assert_eq!(st, 400, "重复编码必须 400: {v}");
    assert_eq!(v["msg"], "字典编码已存在", "{v}");
    assert_eq!(v["err"], "dict.code_taken", "{v}");

    // 编码字符集/长度
    for bad in ["USER_SEX", "user-sex", "a", "用_户", ""] {
        let (st, v) = call(&c, Method::POST, api("/dicts"), Some(&admin), Some(json!({
            "name": "非法编码", "code": bad, "status": 1,
        }))).await;
        assert_eq!(st, 400, "编码 {bad:?} 必须 400: {v}");
        assert_eq!(v["msg"], "字典编码只能是小写字母、数字、下划线，长度 2~64", "{v}");
        assert_eq!(v["err"], "dict.code_format", "格式校验也要能被前端翻译: {v}");
    }
    let (st, v) = call(&c, Method::POST, api("/dicts"), Some(&admin), Some(json!({
        "name": "  ", "code": "ok_code", "status": 1,
    }))).await;
    assert_eq!(st, 400, "空名称 400: {v}");

    // 更新：name/status/remark 生效；body 里带的 code 被忽略（code 不可改）
    let (st, v) = call(&c, Method::PUT, api(&format!("/dicts/{sex_id}")), Some(&admin), Some(json!({
        "name": "用户性别(改)", "status": 1, "remark": "改过", "code": "hacked",
    }))).await;
    assert_eq!(st, 200, "更新类型: {v}");
    let (_st, v) = call(&c, Method::GET, api(&format!("/dicts?name=用户性别")), Some(&admin), None).await;
    assert_eq!(v["data"]["list"][0]["code"], "user_sex", "code 不可改（请求里的 hacked 被忽略）: {v}");
    assert_eq!(v["data"]["list"][0]["remark"], "改过", "{v}");

    let (st, v) = call(&c, Method::PUT, api("/dicts/999999"), Some(&admin), Some(json!({
        "name": "不存在", "status": 1,
    }))).await;
    assert_eq!(st, 404, "更新不存在的类型: {v}");

    // ── 字典项 CRUD + `{code}/items` 下拉 ───────────────────
    let mk_item = |code: &str, label: &str, value: &str, sort: i32, status: i8| {
        json!({"type_code": code, "label": label, "value": value, "sort": sort, "status": status})
    };
    let mut item_ids = Vec::new();
    for (label, value, sort, status) in
        [("女", "0", 2, 1), ("男", "1", 1, 1), ("未知", "2", 3, 0)]
    {
        let (st, v) = call(&c, Method::POST, api("/dict-items"), Some(&admin),
            Some(mk_item("user_sex", label, value, sort, status))).await;
        assert_eq!(st, 200, "建字典项 {label}: {v}");
        item_ids.push(v["data"]["id"].as_u64().unwrap());
    }
    // 另一个类型下的同名 value：证明唯一键是 (type_code, value) 而不是全局
    // （先建成停用：顺手验「条目全停用」也是 200 + []）
    let (st, v) = call(&c, Method::POST, api("/dict-items"), Some(&admin),
        Some(mk_item("order_status", "待付款", "1", 1, 0))).await;
    assert_eq!(st, 200, "别的类型下同 value 允许: {v}");
    let order_item_id = v["data"]["id"].as_u64().unwrap();
    let (st, v) = call(&c, Method::GET, api("/dicts/order_status/items"), Some(&admin), None).await;
    assert_eq!(st, 200, "条目全停用仍是合法空下拉（不是 404）: {v}");
    assert_eq!(v["data"], json!([]), "{v}");
    let (st, v) = call(&c, Method::PUT, api(&format!("/dict-items/{order_item_id}")), Some(&admin),
        Some(json!({"label": "待付款", "value": "1", "sort": 1, "status": 1}))).await;
    assert_eq!(st, 200, "启用它，供后面用: {v}");

    // 孤儿项：type_code 写错必须 400（不建外键，至少别造出下拉里永远看不到的行）
    let (st, v) = call(&c, Method::POST, api("/dict-items"), Some(&admin),
        Some(mk_item("no_such_code", "孤儿", "9", 0, 1))).await;
    assert_eq!(st, 400, "不存在的 type_code: {v}");
    assert_eq!(v["msg"], "字典类型不存在", "{v}");
    assert_eq!(v["err"], "dict.type_missing", "稳定码要能对上表: {v}");

    // 下拉：只回启用项，按 sort, id 排序，只要 label/value
    let (st, v) = call(&c, Method::GET, api("/dicts/user_sex/items"), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(
        v["data"],
        json!([{"label": "男", "value": "1"}, {"label": "女", "value": "0"}]),
        "只含 status=1 且按 sort 排序（女 sort=2 在男后面，未知停用不出现）: {v}"
    );

    // 普通登录用户（角色没勾任何菜单）也能取下拉
    let (st, v) = call(&c, Method::POST, api("/roles"), Some(&admin), Some(json!({
        "name": "v15_none", "code": "v15_none", "data_scope": 4, "status": 1,
    }))).await;
    assert_eq!(st, 200, "建空角色: {v}");
    let empty_role = v["data"]["id"].as_u64().unwrap();
    let (st, v) = call(&c, Method::POST, api("/admins"), Some(&admin), Some(json!({
        "username": "v15user", "password": "v15user123", "role_ids": [empty_role],
    }))).await;
    assert_eq!(st, 200, "建普通用户: {v}");
    let plain = common::login(&base, "v15user", "v15user123").await.expect("普通用户登录");

    let (st, v) = call(&c, Method::GET, api("/dicts/user_sex/items"), Some(&plain), None).await;
    assert_eq!(st, 200, "下拉不要求 system:dict:list，普通用户可读: {v}");
    assert_eq!(v["data"].as_array().unwrap().len(), 2, "{v}");

    // 管理接口全部 403
    for (m, path, body) in [
        (Method::GET, "/dicts".to_string(), None),
        (Method::POST, "/dicts".to_string(), Some(json!({"name": "x", "code": "xx", "status": 1}))),
        (Method::PUT, format!("/dicts/{sex_id}"), Some(json!({"name": "x", "status": 1}))),
        (Method::DELETE, format!("/dicts/{sex_id}"), None),
        (Method::GET, "/dict-items".to_string(), None),
        (Method::POST, "/dict-items".to_string(),
            Some(mk_item("user_sex", "x", "xx", 0, 1))),
        (Method::PUT, format!("/dict-items/{}", item_ids[0]), Some(json!({"label": "x", "value": "xx"}))),
        (Method::DELETE, format!("/dict-items/{}", item_ids[0]), None),
        (Method::GET, "/dict-items/export".to_string(), None),
    ] {
        let (st, v) = call(&c, m.clone(), api(&path), Some(&plain), body).await;
        assert_eq!(st, 403, "{m} {path} 无权限必须 403: {v}");
        assert!(v["msg"].as_str().unwrap_or("").starts_with("缺少权限"), "{m} {path}: {v}");
        assert_eq!(v["err"], "auth.forbidden", "{m} {path} 403 要带码: {v}");
        assert!(v["args"]["code"].as_str().unwrap_or("").starts_with("system:"),
            "args.code 给出缺的权限码: {m} {path}: {v}");
    }

    // 项列表筛选（管理侧）
    let (st, v) = call(&c, Method::GET, api("/dict-items?type_code=user_sex"), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 3, "该类型全部项（含停用）: {v}");
    let (_st, v) = call(&c, Method::GET, api("/dict-items?type_code=user_sex&status=1"), Some(&admin), None).await;
    assert_eq!(v["data"]["total"], 2, "status 筛选: {v}");
    let (_st, v) = call(&c, Method::GET, api("/dict-items?label=男"), Some(&admin), None).await;
    assert_eq!(v["data"]["total"], 1, "label 模糊筛选: {v}");
    assert_eq!(v["data"]["list"][0]["value"], "1", "{v}");

    // 同类型下 value 重复 → 400（建 / 改两条路径）
    let (st, v) = call(&c, Method::POST, api("/dict-items"), Some(&admin),
        Some(mk_item("user_sex", "重复值", "1", 9, 1))).await;
    assert_eq!(st, 400, "同类型下 value 唯一: {v}");
    assert_eq!(v["msg"], "该类型下字典值已存在", "{v}");
    assert_eq!(v["err"], "dict.value_taken", "{v}");
    let (st, v) = call(&c, Method::PUT, api(&format!("/dict-items/{}", item_ids[0])), Some(&admin),
        Some(json!({"label": "女", "value": "1", "sort": 2, "status": 1}))).await;
    assert_eq!(st, 400, "改成已存在的 value 也要 400: {v}");

    // 值/标签校验
    let (st, v) = call(&c, Method::POST, api("/dict-items"), Some(&admin),
        Some(mk_item("user_sex", "空值", "  ", 0, 1))).await;
    assert_eq!(st, 400, "空 value 400: {v}");
    assert_eq!(v["msg"], "字典值不能为空", "{v}");
    assert_eq!(v["err"], "dict.value_required", "{v}");
    let (st, v) = call(&c, Method::POST, api("/dict-items"), Some(&admin),
        Some(mk_item("user_sex", "  ", "9", 0, 1))).await;
    assert_eq!(st, 400, "空 label 400: {v}");
    assert_eq!(v["msg"], "字典标签不能为空", "{v}");
    assert_eq!(v["err"], "dict.label_required", "{v}");
    let long = "x".repeat(65);
    let (st, v) = call(&c, Method::POST, api("/dict-items"), Some(&admin),
        Some(mk_item("user_sex", "超长", &long, 0, 1))).await;
    assert_eq!(st, 400, "value 超 64: {v}");

    // 更新生效（label/sort/status），type_code 提交了也忽略
    let (st, v) = call(&c, Method::PUT, api(&format!("/dict-items/{}", item_ids[2])), Some(&admin),
        Some(json!({"label": "保密", "value": "2", "sort": 0, "status": 1,
                    "type_code": "order_status"}))).await;
    assert_eq!(st, 200, "更新字典项: {v}");
    let (_st, v) = call(&c, Method::GET, api("/dicts/user_sex/items"), Some(&admin), None).await;
    assert_eq!(
        v["data"],
        json!([{"label": "保密", "value": "2"}, {"label": "男", "value": "1"}, {"label": "女", "value": "0"}]),
        "启用 + 改过 sort 后按 sort 重排；type_code 未被改动: {v}"
    );

    // ── 导出：BOM + 表头 + 行数与库一致（筛选与列表同源）──
    let lines = csv_lines(&c, api("/dict-items/export?type_code=user_sex"), &admin).await;
    assert_eq!(lines[0], "ID,类型编码,标签,值,排序,状态,备注,创建时间", "表头: {}", lines[0]);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dict_item WHERE type_code = 'user_sex'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(lines.len() as i64, n + 1, "表头 + 全部 {n} 行: {lines:?}");
    assert!(lines.iter().skip(1).all(|l| l.contains("user_sex")), "筛选生效: {lines:?}");

    let lines = csv_lines(&c, api("/dict-items/export?type_code=user_sex&status=1"), &admin).await;
    assert_eq!(lines.len(), 4, "status=1 只剩 3 行 + 表头: {lines:?}");

    // ── 级联删除：删类型连带删它的项，别处不受影响 ────────────
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dict_item WHERE type_code = 'user_sex'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, 3, "删前 3 项");
    let (st, v) = call(&c, Method::DELETE, api(&format!("/dicts/{sex_id}")), Some(&admin), None).await;
    assert_eq!(st, 200, "删类型: {v}");
    assert_eq!(v["data"]["items_deleted"], 3, "级联删除的项数: {v}");
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dict_item WHERE type_code = 'user_sex'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after, 0, "级联后该类型无残留项");
    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dict_item WHERE type_code = 'order_status'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kept, 1, "别类型的项不能跟着被删");
    let (st, v) = call(&c, Method::GET, api("/dicts/user_sex/items"), Some(&admin), None).await;
    assert_eq!(st, 404, "类型被删后 code 就不存在了，下拉要 404（不是空数组）: {v}");
    assert_eq!(v["msg"], "字典类型不存在", "{v}");
    assert_eq!(v["err"], "dict.type_missing", "稳定码要能对上表: {v}");
    let (st, v) = call(&c, Method::DELETE, api(&format!("/dicts/{sex_id}")), Some(&admin), None).await;
    assert_eq!(st, 404, "再删一次 404: {v}");

    // 字典项删除 + 不存在 404（user_sex 的项已随类型级联删掉，这里是另一个类型的项）
    let (st, v) = call(&c, Method::DELETE, api(&format!("/dict-items/{order_item_id}")), Some(&admin), None).await;
    assert_eq!(st, 200, "删项: {v}");
    let (st, v) = call(&c, Method::DELETE, api(&format!("/dict-items/{order_item_id}")), Some(&admin), None).await;
    assert_eq!(st, 404, "重复删 404: {v}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dict_item").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0, "级联 + 手动删之后字典项归零: {n}");

    // 删类型的操作要进审计（module=dict，动作有中文名）
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE module = 'dict' AND action = '删除字典类型'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    // 3 = 普通用户被 403 拒的那次 + 超管删成功那次 + 重复删 404 那次（失败写操作也留痕）
    assert_eq!(n, 3, "删类型全部留痕: {n}");

    // 清场：剩下的类型删掉，重开进程验证菜单补齐是幂等的（不重复建字典菜单）
    let (st, v) = call(&c, Method::DELETE, api(&format!("/dicts/{order_id}")), Some(&admin), None).await;
    assert_eq!(st, 200, "删第二个类型: {v}");
    let (_server2, base2) = common::start_server().await;
    let admin2 = common::login(&base2, "admin", "admin123").await.expect("重启后登录");
    let (st, v) = call(&c, Method::GET, format!("{base2}/api/v1/menus/tree"), Some(&admin2), None).await;
    assert_eq!(st, 200, "{v}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM menu WHERE perm = 'system:dict:list'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "重启补齐幂等，字典菜单只有一条: {n}");
    assert!(menu(&v["data"], "system:dict:add").is_some(), "按钮码也还在: {v}");
    let (st, v) = call(&c, Method::GET, format!("{base2}/api/v1/dict-items"), Some(&admin2), None).await;
    assert_eq!(st, 200, "重启后接口可用: {v}");
    assert_eq!(v["data"]["total"], 0, "两个类型都删干净了: {v}");
}
