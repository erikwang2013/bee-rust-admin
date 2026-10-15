// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 扩展工程 A3（项 1）系统配置（sys_config）：公开读免登录 / 管理端 CRUD / 权限码生效 /
//! public 不泄露非公开键（负控）/ 写路径审计留痕。
//! 起真实进程 + 真库（bee_admin_test），需要 `BEE_ADMIN_DB_DSN`。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

use common::{dsn, login, reset_db, start_server};

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

/// 菜单树里按权限码取菜单 id（对外 id 是 hashid 串，原样透传）。
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

const CONFIGS: &str = "/api/v1/configs";

#[tokio::test]
async fn sys_config_and_public() {
    let Some(dsn) = dsn() else { return };
    reset_db(&dsn).await;
    let (_server, base) = start_server().await;
    let c = reqwest::Client::new();

    // ── 公开读（免登录）：种子里的公开键（site 4 个）；bool 按类型转 JSON 布尔而不是字符串 ──
    let (st, v) = call(&c, Method::GET, format!("{base}{CONFIGS}/public"), None, None).await;
    assert_eq!(st, 200, "公开配置不登录也要通: {v}");
    let pub_data = &v["data"];
    assert_eq!(
        pub_data["site.repo_url"], "https://github.com/erikwang2013/bee-rust-admin",
        "项目地址默认值: {v}"
    );
    assert_eq!(pub_data["site.footer_enabled"], json!(true), "bool 转 JSON 布尔: {v}");
    assert_eq!(pub_data["site.login_subtitle"], "", "副标题默认空: {v}");
    let copyright = pub_data["site.copyright"].as_str().unwrap_or("<缺 site.copyright>");
    assert!(copyright.contains("erik"), "版权默认值: {v}");

    // ── 管理端列表（超管）──
    let admin = login(&base, "admin", "admin123").await.expect("超管登录");
    let (st, v) = call(&c, Method::GET, format!("{base}{CONFIGS}"), Some(&admin), None).await;
    assert_eq!(st, 200, "超管可读配置列表: {v}");
    let list = v["data"]["list"].as_array().expect("list 数组").clone();
    assert_eq!(v["data"]["total"], json!(list.len()), "total 与 list 一致: {v}");
    // 种子 13 个键（site 4 + 通知 9），但列表**只回 12**：项 6 的 SMTP 口令键
    // 被 `is_secret_key` 挡在管理页外（列出来等于让每个能读配置的人改它的值，
    // 「加密存」就白做了）。口令键的存在只在「通知设置」页看得到。
    assert_eq!(list.len(), 12, "种子 13 个键 - 1 个口令键: {v}");
    assert!(
        !list.iter().any(|x| x["key"] == "notify.smtp_password"),
        "口令键不上通用配置页: {v}"
    );
    // 通知键一律私有：`/configs/public` 免登录，公开等于把内部地址抖出去
    let (_, v) = call(&c, Method::GET, format!("{base}{CONFIGS}/public"), None, None).await;
    assert!(
        !v["data"].as_object().unwrap().keys().any(|k| k.starts_with("notify.")),
        "通知配置不许公开读: {v}"
    );
    let find = |rows: &[Value], k: &str| -> Value {
        rows.iter().find(|x| x["key"] == k).cloned().unwrap_or_else(|| panic!("缺 {k}: {v}"))
    };
    let sub = find(&list, "site.login_subtitle");
    assert_eq!(sub["group"], "site");
    assert_eq!(sub["value_type"], "string");
    assert_eq!(sub["is_public"], 1);
    assert_eq!(find(&list, "site.footer_enabled")["value_type"], "bool");

    // ── 改配置 → 公开读立即可见（验收：改配置 → 登录页/页脚刷新即变）──
    let sub_id = common::as_id(&sub["id"]);
    let (st, v) = call(
        &c, Method::PUT, format!("{base}{CONFIGS}/{sub_id}"), Some(&admin),
        Some(json!({"value": "欢迎使用"})),
    )
    .await;
    assert_eq!(st, 200, "改值: {v}");
    let (_, v) = call(&c, Method::GET, format!("{base}{CONFIGS}/public"), None, None).await;
    assert_eq!(v["data"]["site.login_subtitle"], "欢迎使用", "公开读不该有缓存: {v}");

    // 部分更新：只改 value 不动 value_type / is_public
    let footer_id = common::as_id(&find(&list, "site.footer_enabled")["id"]);
    let (st, v) = call(
        &c, Method::PUT, format!("{base}{CONFIGS}/{footer_id}"), Some(&admin),
        Some(json!({"value": "false"})),
    )
    .await;
    assert_eq!(st, 200, "改 bool 值: {v}");
    let (_, v) = call(&c, Method::GET, format!("{base}{CONFIGS}"), Some(&admin), None).await;
    let after = v["data"]["list"].as_array().unwrap().clone();
    let f = find(&after, "site.footer_enabled");
    assert_eq!(f["value_type"], "bool", "只传 value 不能把类型归位: {v}");
    assert_eq!(f["is_public"], 1, "只传 value 不能把公开标记归位: {v}");
    let (_, v) = call(&c, Method::GET, format!("{base}{CONFIGS}/public"), None, None).await;
    assert_eq!(v["data"]["site.footer_enabled"], json!(false), "false 转 JSON false: {v}");

    // ── 新建默认私有（fail-safe）；public 不泄露非公开键（负控）──
    let (st, v) = call(
        &c, Method::POST, format!("{base}{CONFIGS}"), Some(&admin),
        Some(json!({"key": "site.internal_note", "value": "hunter2"})),
    )
    .await;
    assert_eq!(st, 200, "新建: {v}");
    let note_id = common::as_id(&v["data"]["id"]);
    let (_, v) = call(&c, Method::GET, format!("{base}{CONFIGS}"), Some(&admin), None).await;
    let n = find(&v["data"]["list"].as_array().unwrap().clone(), "site.internal_note");
    assert_eq!(n["is_public"], 0, "新建默认私有，别让人手滑外泄: {v}");
    let (_, v) = call(&c, Method::GET, format!("{base}{CONFIGS}/public"), None, None).await;
    assert!(v["data"]["site.internal_note"].is_null(), "非公开键绝不能出现在 public 里: {v}");

    // 显式公开（is_public=1 整数 —— 前端 Switch 的布尔在提交时归一成 0/1，见配置页）：
    // 建完要立刻出现在 public 里，这是「新增公开键」这条 UI 路径的线上契约
    let (st, v) = call(
        &c, Method::POST, format!("{base}{CONFIGS}"), Some(&admin),
        Some(json!({"key": "site.public_marker", "value": "on", "is_public": 1})),
    )
    .await;
    assert_eq!(st, 200, "显式公开键: {v}");
    let marker_id = common::as_id(&v["data"]["id"]);
    let (_, v) = call(&c, Method::GET, format!("{base}{CONFIGS}/public"), None, None).await;
    assert_eq!(v["data"]["site.public_marker"], "on", "显式公开的键要出现在 public: {v}");
    let (st, _) = call(&c, Method::DELETE, format!("{base}{CONFIGS}/{marker_id}"), Some(&admin), None).await;
    assert_eq!(st, 200, "清理公开标记键");

    // 校验：坏 key / 太短 key / 坏类型 / 重复 key 全 400
    for body in [
        json!({"key": "Bad Key", "value": "x"}),
        json!({"key": "x", "value": "x"}),
        json!({"key": "site.tmp", "value": "x", "value_type": "weird"}),
        json!({"key": "site.internal_note", "value": "again"}),
    ] {
        let (st, v) = call(&c, Method::POST, format!("{base}{CONFIGS}"), Some(&admin), Some(body.clone())).await;
        assert_eq!(st, 400, "该 400: {body} → {v}");
    }

    // ── 删除：删掉即消失，再删 404 ──
    let (st, v) = call(&c, Method::DELETE, format!("{base}{CONFIGS}/{note_id}"), Some(&admin), None).await;
    assert_eq!(st, 200, "删除: {v}");
    let (st, _) = call(&c, Method::DELETE, format!("{base}{CONFIGS}/{note_id}"), Some(&admin), None).await;
    assert_eq!(st, 404, "再删 404");

    // ── 权限码生效：有 list 无 edit 只能读；补上 edit 才能改（权限每请求从库重算）──
    let (_, v) = call(&c, Method::GET, format!("{base}/api/v1/menus/tree"), Some(&admin), None).await;
    let list_menu = menu_id(&v["data"], "system:config:list").expect("种子里有系统配置菜单");
    let edit_menu = menu_id(&v["data"], "system:config:edit").expect("种子里有编辑按钮");

    let (_, v) = call(
        &c, Method::POST, format!("{base}/api/v1/roles"), Some(&admin),
        Some(json!({"name": "cfg_viewer", "code": "cfg_viewer", "data_scope": 1, "status": 1})),
    )
    .await;
    let role_id = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c, Method::PUT, format!("{base}/api/v1/roles/{role_id}/menus"), Some(&admin),
        Some(json!({"menu_ids": [list_menu]})),
    )
    .await;
    assert_eq!(st, 200, "挂 list 菜单: {v}");
    let (st, v) = call(
        &c, Method::POST, format!("{base}/api/v1/admins"), Some(&admin),
        Some(json!({"username": "cfgv", "password": "cfgv12345", "role_ids": [role_id]})),
    )
    .await;
    assert_eq!(st, 200, "建受限用户: {v}");

    let viewer = login(&base, "cfgv", "cfgv12345").await.expect("受限用户登录");
    let (st, v) = call(&c, Method::GET, format!("{base}{CONFIGS}"), Some(&viewer), None).await;
    assert_eq!(st, 200, "有 list 码能读: {v}");
    let (st, v) = call(
        &c, Method::PUT, format!("{base}{CONFIGS}/{sub_id}"), Some(&viewer),
        Some(json!({"value": "被篡改"})),
    )
    .await;
    assert_eq!(st, 403, "没有 edit 码不能改: {v}");
    let (_, v) = call(&c, Method::GET, format!("{base}{CONFIGS}/public"), None, None).await;
    assert_eq!(v["data"]["site.login_subtitle"], "欢迎使用", "403 的改动没落库: {v}");

    // 补上 edit 菜单后同一个 token 即可改（权限码每请求从库重算）
    let (st, v) = call(
        &c, Method::PUT, format!("{base}/api/v1/roles/{role_id}/menus"), Some(&admin),
        Some(json!({"menu_ids": [list_menu, edit_menu]})),
    )
    .await;
    assert_eq!(st, 200, "补 edit 菜单: {v}");
    let (st, v) = call(
        &c, Method::PUT, format!("{base}{CONFIGS}/{sub_id}"), Some(&viewer),
        Some(json!({"value": "欢迎使用 bee-rust"})),
    )
    .await;
    assert_eq!(st, 200, "有 edit 码就能改: {v}");

    // 什么都没挂的人：读也 403
    let (_, v) = call(
        &c, Method::POST, format!("{base}/api/v1/roles"), Some(&admin),
        Some(json!({"name": "cfg_none", "code": "cfg_none", "data_scope": 1, "status": 1})),
    )
    .await;
    let none_role = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c, Method::POST, format!("{base}/api/v1/admins"), Some(&admin),
        Some(json!({"username": "cfgn", "password": "cfgn12345", "role_ids": [none_role]})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    let none = login(&base, "cfgn", "cfgn12345").await.expect("无权限用户登录");
    let (st, v) = call(&c, Method::GET, format!("{base}{CONFIGS}"), Some(&none), None).await;
    assert_eq!(st, 403, "无 list 码读配置也必须 403: {v}");

    // ── 写操作留痕：审计里有 config 模块的记录，动作名点名系统配置 ──
    let (st, v) = call(
        &c, Method::GET, format!("{base}/api/v1/audit-logs?module=config&size=100"), Some(&admin), None,
    )
    .await;
    assert_eq!(st, 200, "审计列表: {v}");
    let rows = v["data"]["list"].as_array().expect("审计 list");
    assert!(!rows.is_empty(), "配置写路径要有审计留痕: {v}");
    let actions: Vec<&str> = rows.iter().filter_map(|r| r["action"].as_str()).collect();
    assert!(actions.iter().any(|a| a.contains("系统配置")), "动作名: {actions:?}");
}
