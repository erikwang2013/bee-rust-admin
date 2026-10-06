// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! BRD v1.2 后端新能力集成测试：B2 禁用菜单即收权 / B3 提取器信封 / B4 登录锁定
//! （含两处语义修正：成功登录清失败窗口、按 IP 限流）/ B5 操作日志 / B6 CSV 导出 /
//! B7 个人资料 / B8 头像 / B9 菜单幂等补齐。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use base64::Engine;
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

/// 带固定 `X-Real-IP` 的登录请求（IP 限流用例；`call()` 不带这个头，落 "unknown" 桶）。
async fn login_from(
    c: &reqwest::Client,
    url: &str,
    ip: &str,
    user: &str,
    pass: &str,
) -> (u16, Value) {
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

/// 取 CSV 响应体（含 BOM），返回 (body 原文, 去掉 BOM 的文本行)。
async fn csv_rows(c: &reqwest::Client, url: String, token: &str) -> (Vec<u8>, Vec<String>) {
    let r = c.get(url).bearer_auth(token).send().await.expect("导出请求失败");
    assert_eq!(r.status(), 200, "导出必须 200");
    assert_eq!(
        r.headers()["content-type"],
        "text/csv; charset=utf-8",
        "导出 Content-Type"
    );
    let cd = r.headers()["content-disposition"].to_str().unwrap().to_string();
    assert!(cd.contains("attachment") && cd.contains(".csv"), "附件头: {cd}");
    let body = r.bytes().await.unwrap().to_vec();
    let text = String::from_utf8(body[3..].to_vec()).expect("BOM 之后是 UTF-8");
    (body, text.trim_end().split("\r\n").map(str::to_string).collect())
}

/// 递归收集菜单树里的 perm（含按钮节点）。
fn collect_perms(menus: &Value, out: &mut Vec<String>) {
    for m in menus.as_array().cloned().unwrap_or_default() {
        out.push(m["perm"].as_str().unwrap_or("").to_string());
        collect_perms(&m["children"], out);
    }
}

/// 1x1 PNG 的 data URL（真实文件字节，非伪造魔数）。
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

#[tokio::test]
async fn v12_backend_features() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    // 头像落盘目录：测试前清干净，避免上一轮运行留下的文件干扰 404 断言
    let _ = std::fs::remove_dir_all("target/test-uploads");

    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录失败");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── B7 个人资料 ─────────────────────────────────────────
    let (st, v) = call(&c, Method::PUT, api("/auth/profile"), Some(&admin),
        Some(json!({"nickname": "新昵称", "email": "a@b.c", "phone": "13800000000"}))).await;
    assert_eq!(st, 200, "改资料: {v}");
    let (st, v) = call(&c, Method::GET, api("/auth/profile"), Some(&admin), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["user"]["nickname"], "新昵称", "回读一致: {v}");
    assert_eq!(v["data"]["user"]["email"], "a@b.c", "邮箱回读: {v}");
    assert_eq!(v["data"]["user"]["phone"], "13800000000", "手机号回读: {v}");
    // …而且真的落库了（GET 回填的字段必须能原样 PUT 回去，否则表单会静默清数据）
    let (db_email, db_phone): (String, String) =
        sqlx::query_as("SELECT email, phone FROM admin WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((db_email.as_str(), db_phone.as_str()), ("a@b.c", "13800000000"), "库里与回读一致");
    // 改资料不作废 token（改个昵称不该被踢下线）：上面的 admin token 一直有效
    let (st, v) = call(&c, Method::PUT, api("/auth/profile"), Some(&admin),
        Some(json!({"nickname": "角".repeat(65)}))).await;
    assert_eq!(st, 400, "超长昵称应 400: {v}");
    assert_eq!(v["code"], 400, "错误响应必须是 JSON 信封: {v}");

    // ── B8 头像上传 / 读取 ──────────────────────────────────
    let png = base64::engine::general_purpose::STANDARD.decode(PNG_B64).unwrap();
    let (st, v) = call(&c, Method::POST, api("/auth/avatar"), Some(&admin),
        Some(json!({ "data_url": format!("data:image/png;base64,{PNG_B64}") }))).await;
    assert_eq!(st, 200, "上传头像: {v}");
    assert_eq!(v["data"]["avatar"], "/api/v1/avatar/1", "头像契约路径: {v}");

    let r = c.get(api("/avatar/1")).send().await.unwrap();
    assert_eq!(r.status(), 200, "头像读取公开（<img> 带不了 token）");
    assert_eq!(r.headers()["content-type"], "image/png", "Content-Type 由扩展名决定");
    assert_eq!(r.bytes().await.unwrap().as_ref(), png.as_slice(), "读回字节与上传一致");

    let (st, v) = call(&c, Method::POST, api("/auth/avatar"), Some(&admin),
        Some(json!({"data_url": "data:image/png;base64,QUJD"}))).await; // "ABC"
    assert_eq!(st, 400, "非图片（魔数不符）应 400: {v}");
    assert!(v["msg"].as_str().unwrap().contains("图片"), "错误说明: {v}");
    let (st, v) = call(&c, Method::POST, api("/auth/avatar"), Some(&admin),
        Some(json!({"data_url": "data:image/gif;base64,R0lGODlhAQABAAAAAA=="}))).await;
    assert_eq!(st, 400, "不支持的类型应 400: {v}");
    let r = c.get(api("/avatar/999")).send().await.unwrap();
    assert_eq!(r.status(), 404, "没有头像的管理员 → 404");

    // ── B3 提取器也要回信封 ─────────────────────────────────
    let r = c.get(api("/admins?page=abc")).bearer_auth(&admin).send().await.unwrap();
    assert_eq!(r.status(), 400, "非法查询参数");
    assert_eq!(r.headers()["content-type"], "application/json", "不是 axum 的纯文本 400");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["code"], 400, "查询参数错误信封: {v}");
    let r = c.get(api("/admins/abc")).bearer_auth(&admin).send().await.unwrap();
    assert_eq!(r.status(), 400, "非法路径参数");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["code"], 400, "路径参数错误信封: {v}");

    // ── B5 操作日志 ─────────────────────────────────────────
    let (st, v) = call(&c, Method::POST, api("/admins"), Some(&admin),
        Some(json!({"username": "auditee", "password": "auditee123", "nickname": "被审计"}))).await;
    assert_eq!(st, 200, "建管理员: {v}");

    let (st, v) = call(&c, Method::GET, api("/audit-logs?module=admin"), Some(&admin), None).await;
    assert_eq!(st, 200, "审计列表: {v}");
    assert!(v["data"]["total"].as_u64().unwrap() >= 1, "新增管理员必须留痕: {v}");
    let row = &v["data"]["list"][0];
    assert_eq!(row["module"], "admin");
    assert_eq!(row["action"], "新增管理员");
    assert_eq!(row["status"], 1, "2xx 记成功: {row}");
    assert_eq!(row["username"], "admin", "谁操作的一目了然: {row}");
    assert_eq!(row["method"], "POST");
    assert_eq!(row["path"], "/api/v1/admins");
    assert!(row["duration_ms"].is_u64(), "耗时: {row}");
    assert!(row["ip"].is_string(), "IP: {row}");
    assert_eq!(row["created_at"].as_str().unwrap().len(), 19, "时间格式 2026-10-06 02:40:00: {row}");

    // 失败也留痕，msg 取自错误信封
    let (st, v) = call(&c, Method::GET, api("/audit-logs?status=0&module=auth"), Some(&admin), None).await;
    assert_eq!(st, 200);
    let failed = v["data"]["list"].as_array().unwrap();
    let profile_fail = failed.iter().find(|r| r["path"] == "/api/v1/auth/profile")
        .unwrap_or_else(|| panic!("改资料的失败也要留痕: {v}"));
    assert_eq!(profile_fail["status"], 0);
    assert!(profile_fail["msg"].as_str().unwrap().contains("昵称"), "失败摘要来自信封 msg: {v}");

    // 登录不记操作日志（它有自己的 login_log）
    let (st, v) = call(&c, Method::GET, api("/audit-logs?size=100"), Some(&admin), None).await;
    assert_eq!(st, 200);
    assert!(
        v["data"]["list"].as_array().unwrap().iter().all(|r| r["path"] != "/api/v1/auth/login"),
        "登录不该进操作日志: {v}"
    );

    // 无权限者看不到审计日志
    let (st, v) = call(&c, Method::POST, api("/roles"), Some(&admin),
        Some(json!({"name": "只读", "code": "readonly", "data_scope": 4}))).await;
    assert_eq!(st, 200, "建角色: {v}");
    let readonly_role = v["data"]["id"].as_u64().unwrap();
    let (st, _v) = call(&c, Method::POST, api("/admins"), Some(&admin), Some(json!({
        "username": "peeper", "password": "peeper123", "role_ids": [readonly_role]
    }))).await;
    assert_eq!(st, 200);
    let peeper = common::login(&base, "peeper", "peeper123").await.expect("peeper 登录");
    let (st, v) = call(&c, Method::GET, api("/audit-logs"), Some(&peeper), None).await;
    assert_eq!(st, 403, "无 system:auditlog:list 必须 403: {v}");

    // 清空：动作本身也被记一条（写操作）
    let (st, v) = call(&c, Method::DELETE, api("/audit-logs?module=auth"), Some(&admin), None).await;
    assert_eq!(st, 200, "清空审计: {v}");
    assert!(v["data"]["deleted"].as_u64().unwrap() >= 3, "改资料×2 + 上传头像×2: {v}");
    let (st, v) = call(&c, Method::GET, api("/audit-logs?module=auth"), Some(&admin), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["total"], 0, "auth 模块已清空: {v}");
    let (st, v) = call(&c, Method::GET, api("/audit-logs?module=auditlog"), Some(&admin), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["total"], 1, "清空动作自己留了一条: {v}");
    assert_eq!(v["data"]["list"][0]["action"], "清空操作日志");

    // ── B6 CSV 导出 ────────────────────────────────────────
    // 造一个「公式开头」的昵称，验证 CSV 注入防护
    let (st, _v) = call(&c, Method::POST, api("/admins"), Some(&admin),
        Some(json!({"username": "csvuser", "password": "csvuser123", "nickname": "=1+1"}))).await;
    assert_eq!(st, 200);

    let (body, lines) = csv_rows(&c, api("/admins/export"), &admin).await;
    assert!(body.starts_with(&[0xEF, 0xBB, 0xBF]), "首字节必须是 UTF-8 BOM");
    assert!(lines[0].starts_with("ID,用户名,昵称,部门,角色"), "表头: {}", lines[0]);
    assert_eq!(lines.len(), 5, "表头 + 4 个管理员（admin/auditee/peeper/csvuser）: {lines:?}");
    assert!(lines[1..].iter().any(|l| l.contains("'=1+1")), "公式开头要前置单引号: {lines:?}");

    let (body, lines) = csv_rows(&c, api("/login-logs/export?username=admin"), &admin).await;
    assert!(body.starts_with(&[0xEF, 0xBB, 0xBF]), "BOM");
    assert!(lines[0].starts_with("ID,用户名,IP"), "表头: {}", lines[0]);
    assert_eq!(lines.len(), 2, "admin 只登录过一次: {lines:?}");

    let (body, lines) = csv_rows(&c, api("/audit-logs/export?module=auditlog"), &admin).await;
    assert!(body.starts_with(&[0xEF, 0xBB, 0xBF]), "BOM");
    assert!(lines[0].starts_with("ID,时间,用户,模块,动作"), "表头: {}", lines[0]);
    assert_eq!(lines.len(), 2, "一条「清空操作日志」: {lines:?}");

    // 导出复用 list 权限码
    let (st, v) = call(&c, Method::GET, api("/audit-logs/export"), Some(&peeper), None).await;
    assert_eq!(st, 403, "导出复用 list 权限: {v}");

    // ── B2 禁用菜单即收权 ───────────────────────────────────
    let (st, v) = call(&c, Method::GET, api("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200);
    let dept_menu = v["data"][0]["children"].as_array().unwrap()
        .iter().find(|m| m["name"] == "部门管理").expect("种子含「部门管理」").clone();
    let dept_menu_id = dept_menu["id"].as_u64().unwrap();

    let (st, _v) = call(&c, Method::PUT, api(&format!("/roles/{readonly_role}/menus")), Some(&admin),
        Some(json!({"menu_ids": [dept_menu_id]}))).await;
    assert_eq!(st, 200);
    let (st, _v) = call(&c, Method::POST, api("/admins"), Some(&admin),
        Some(json!({"username": "gate1", "password": "gate12345", "role_ids": [readonly_role]}))).await;
    assert_eq!(st, 200);
    let gate = common::login(&base, "gate1", "gate12345").await.expect("gate1 登录");

    let (st, v) = call(&c, Method::GET, api("/auth/profile"), Some(&gate), None).await;
    assert_eq!(st, 200, "gate1 的资料: {v}");
    let perms: Vec<String> = v["data"]["perms"].as_array().unwrap()
        .iter().map(|p| p.as_str().unwrap().to_string()).collect();
    assert!(perms.contains(&"system:dept:list".to_string()), "启用时权限码在: {perms:?}");
    let (st, v) = call(&c, Method::GET, api("/depts/tree"), Some(&gate), None).await;
    assert_eq!(st, 200, "有权限时部门树可读: {v}");

    // 超管把「部门管理」菜单置 0（角色勾选关系不动）
    let mut disabled = dept_menu.clone();
    disabled["status"] = json!(0);
    let (st, v) = call(&c, Method::PUT, api(&format!("/menus/{dept_menu_id}")), Some(&admin), Some(disabled)).await;
    assert_eq!(st, 200, "禁用菜单: {v}");

    let (st, v) = call(&c, Method::GET, api("/auth/profile"), Some(&gate), None).await;
    assert_eq!(st, 200, "gate1 的资料: {v}");
    let perms: Vec<String> = v["data"]["perms"].as_array().unwrap()
        .iter().map(|p| p.as_str().unwrap().to_string()).collect();
    assert!(!perms.contains(&"system:dept:list".to_string()), "禁用后权限码消失: {perms:?}");
    let (st, v) = call(&c, Method::GET, api("/depts/tree"), Some(&gate), None).await;
    assert_eq!(st, 403, "禁用后对应接口 403: {v}");

    // ── B4 登录失败锁定（security-rust throttle，状态在进程内存里）──
    let (st, _v) = call(&c, Method::POST, api("/admins"), Some(&admin),
        Some(json!({"username": "lockme", "password": "lockme123"}))).await;
    assert_eq!(st, 200);
    for i in 1..=5 {
        let (st, v) = call(&c, Method::POST, api("/auth/login"), None,
            Some(json!({"username": "lockme", "password": "wrong"}))).await;
        assert_eq!(st, 400, "第 {i} 次错密码: {v}");
        assert!(v["msg"].as_str().unwrap().contains("用户名或密码错误"),
            "未达阈值前仍提示密码错（不暴露锁定逻辑）: {v}");
        assert_eq!(v["err"], "auth.bad_credentials", "{v}");
    }
    // 第 5 次失败当场记入封禁，但这一发的响应已经定了 —— 下一个请求才吃 429
    let (st, v) = call(&c, Method::POST, api("/auth/login"), None,
        Some(json!({"username": "lockme", "password": "lockme123"}))).await;
    assert_eq!(st, 429, "封禁后正确密码也拒绝: {v}");
    assert!(v["msg"].as_str().unwrap().contains("频繁"), "提示要说明限流: {v}");
    assert_eq!(v["err"], "auth.throttled", "{v}");
    assert_eq!(v["args"]["minutes"], 10, "带 N 分钟的码要给原始值: {v}");
    assert!(common::login(&base, "lockme", "lockme123").await.is_none(), "封禁期内登不进去");

    // 另一个用户不受影响（账号维度按用户名分桶）。这一发同时也验证了直连不做 IP 维度：
    // 无 X-Real-IP 时所有人都是 "unknown"，若照收进 IP 桶，它此刻已有 lockme 的 5 次失败
    assert!(common::login(&base, "admin", "admin123").await.is_some(),
        "锁定只针对该账号；直连（无 IP 头）也不该被并进同一个 IP 桶");

    // 修 (a)：成功登录清掉该账号的失败计数
    let (st, _v) = call(&c, Method::POST, api("/admins"), Some(&admin),
        Some(json!({"username": "clearwind", "password": "clearwind123"}))).await;
    assert_eq!(st, 200);
    for i in 1..=4 {
        let (st, v) = call(&c, Method::POST, api("/auth/login"), None,
            Some(json!({"username": "clearwind", "password": "wrong"}))).await;
        assert_eq!(st, 400, "第 {i} 次错密码（未达阈值 5）: {v}");
    }
    assert!(common::login(&base, "clearwind", "clearwind123").await.is_some(), "4 次失败后本人登录成功");
    let (st, v) = call(&c, Method::POST, api("/auth/login"), None,
        Some(json!({"username": "clearwind", "password": "wrong"}))).await;
    assert_eq!(st, 400, "成功后再错一次: {v}");
    assert!(v["msg"].as_str().unwrap().contains("用户名或密码错误"),
        "这一发不该被算成封禁: {v}");
    assert!(common::login(&base, "clearwind", "clearwind123").await.is_some(),
        "4 失败 + 1 成功 + 1 失败后必须还能登录：成功登录已清零计数");

    // 修 (b)：按 IP 限流。同一条 IP 连续失败（每次换用户名，不触发账号维度）
    for i in 0..5 {
        let (st, v) = login_from(&c, &api("/auth/login"), "1.1.1.1", &format!("ghost{i}"), "nope").await;
        assert_eq!(st, 400, "第 {i} 次（未达阈值）: {v}");
    }
    let (st, v) = login_from(&c, &api("/auth/login"), "1.1.1.1", "admin", "admin123").await;
    assert_eq!(st, 429, "同 IP 失败达阈值后，正确的账号密码也必须 429: {v}");
    assert_eq!(v["code"], 429, "429 也要走 code/msg/data 信封: {v}");
    assert!(v["msg"].as_str().unwrap().contains("频繁"), "提示说明限流: {v}");
    assert_eq!(v["err"], "auth.throttled", "{v}");

    // 换个 IP 不受影响（限流按 IP 分桶）
    let (st, v) = login_from(&c, &api("/auth/login"), "2.2.2.2", "admin", "admin123").await;
    assert_eq!(st, 200, "其他 IP 不受这个桶影响: {v}");

    // login_log 只当历史/审计：被限流拦下的尝试也照写，并留下被尝试的用户名
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_log WHERE ip = '1.1.1.1'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(n, 6, "5 次失败 + 1 次限流拦截: {n}");
    let u: String = sqlx::query_scalar(
        "SELECT username FROM login_log WHERE ip = '1.1.1.1' AND msg LIKE '%频繁%'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(u, "admin", "被拦的尝试照记被尝试的用户名（库=历史，内存=限流）");

    // ── B9 内置菜单幂等补齐 ─────────────────────────────────
    let (st, v) = call(&c, Method::GET, api("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200);
    let mut perms = Vec::new();
    collect_perms(&v["data"], &mut perms);
    assert_eq!(perms.len(), 32, "1 目录 + 9 菜单 + 22 按钮（v1.5 字典 4 个码、v1.6 任务 2 个码 + 公告 4 个码）: {perms:?}");
    assert_eq!(
        perms.iter().filter(|p| *p == "system:auditlog:list").count(), 1,
        "审计日志菜单恰好一条"
    );
    // 删掉审计日志的按钮 + 菜单，重启后应被补齐（既有库也能拿到新菜单）
    let audit_menu = v["data"][0]["children"].as_array().unwrap()
        .iter().find(|m| m["name"] == "操作日志").expect("含「操作日志」菜单").clone();
    let audit_menu_id = audit_menu["id"].as_u64().unwrap();
    let btn_id = audit_menu["children"][0]["id"].as_u64().unwrap();
    for id in [btn_id, audit_menu_id] {
        let (st, v) = call(&c, Method::DELETE, api(&format!("/menus/{id}")), Some(&admin), None).await;
        assert_eq!(st, 200, "删菜单 {id}: {v}");
    }

    // 再起一个进程（同一库）：不产生重复行，且缺的被补回来
    let (_server2, base2) = common::start_server().await;
    let admin2 = common::login(&base2, "admin", "admin123").await.expect("第二次启动仍能登录");
    // 限流状态在进程内存里，换进程就是干净的：lockme 在旧进程里还在封禁期，这里能进。
    // （封禁到期本身由 security-rust 自己的单测覆盖，这里钉的是「每进程一份状态」）
    assert!(common::login(&base2, "lockme", "lockme123").await.is_some(),
        "限流状态不跨进程：重启即清零");
    let (st, v) = call(&c, Method::GET, format!("{base2}/api/v1/menus/tree"), Some(&admin2), None).await;
    assert_eq!(st, 200);
    let mut perms = Vec::new();
    collect_perms(&v["data"], &mut perms);
    assert_eq!(perms.len(), 32, "重启后补齐到 32 行且无重复: {perms:?}");
    assert_eq!(
        perms.iter().filter(|p| *p == "system:auditlog:remove").count(), 1,
        "按钮也补齐且不重复"
    );

    // 超管不被重复种（admin 表已非空）
    let (st, v) = call(&c, Method::GET, format!("{base2}/api/v1/admins?username=admin"), Some(&admin2), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["total"], 1, "超管只有一条: {v}");
}
