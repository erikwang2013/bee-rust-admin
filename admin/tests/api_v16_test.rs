// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! BRA v1.6 后端新能力集成测试：C2 定时任务 + 通知公告。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;

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

/// 起一个附加了额外配置的真实进程（复用 common 的 conf 模板 + 随机端口）。
/// 只为本文件里「[job] enabled=false」这个开关用；其余测试走 `common::start_server`。
async fn start_server_conf(extra: &str) -> (common::Server, String) {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let conf = std::fs::read_to_string("conf/app.conf.test")
        .unwrap()
        .replace("127.0.0.1:0", &format!("127.0.0.1:{port}"));
    let tmp = std::env::temp_dir().join(format!("bee_admin_v16_{port}.conf"));
    std::fs::write(&tmp, format!("{conf}\n{extra}\n")).unwrap();

    let child = std::process::Command::new(env!("CARGO_BIN_EXE_bee_admin"))
        .env("BEE_ADMIN_CONF", &tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("启动 bee_admin 失败");
    let server = common::Server(child);

    let base = format!("http://127.0.0.1:{port}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if reqwest::get(format!("{base}/api/v1/health")).await.is_ok() {
            return (server, base);
        }
        assert!(tokio::time::Instant::now() < deadline, "服务 15 秒内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn job_log_count(pool: &sqlx::MySqlPool, code: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM job_log WHERE job_code = ?")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn v16_job_notice_features() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    // 头像目录会被本测试的孤儿清理动到，先清干净（api_v12 同样做法），让计数确定
    let _ = std::fs::remove_dir_all("target/test-uploads/avatar");

    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");

    // ── 种子菜单：两个新页挂在系统管理下，按钮码齐 ──────────────
    let (st, v) = call(&c, Method::GET, api("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200, "菜单树: {v}");
    let tree = v["data"].clone();
    for (perm, path, comp, buttons) in [
        ("system:job:list", "/system/job", "system/job/index", vec!["edit"]),
        (
            "system:notice:list",
            "/system/notice",
            "system/notice/index",
            vec!["add", "edit", "remove"],
        ),
    ] {
        let m = menu(&tree, perm).unwrap_or_else(|| panic!("种子里必须有 {perm} 菜单: {tree}"));
        assert_eq!(m["path"], path, "页面路径");
        assert_eq!(m["component"], comp, "组件路径");
        let id = common::as_id(&m["id"]);
        let parent_id = common::as_id(&m["parent_id"]);
        let dir = tree
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == json!(parent_id))
            .unwrap_or_else(|| panic!("{perm} 的父节点必须是系统管理: {tree}"));
        assert_eq!(dir["path"], "/system", "{perm} 挂在系统管理下");
        for btn in buttons {
            let bp = format!("{}:{btn}", perm.trim_end_matches(":list"));
            let b = menu(&tree, &bp).unwrap_or_else(|| panic!("缺按钮权限码 {bp}: {tree}"));
            assert_eq!(b["parent_id"], json!(id), "{bp} 挂在该菜单下");
        }
    }
    // 注册表里没有的动作不该长出权限码（job 只有 list/edit）
    assert!(menu(&tree, "system:job:add").is_none(), "job 不该有新增按钮: {tree}");
    assert!(menu(&tree, "system:job:remove").is_none(), "job 不该有删除按钮: {tree}");

    // ── 任务列表：来自代码注册表的幂等补齐 ─────────────────────
    let (st, v) = call(&c, Method::GET, api("/jobs"), Some(&admin), None).await;
    assert_eq!(st, 200, "任务列表: {v}");
    assert_eq!(v["data"]["total"], 2, "内置两个任务: {v}");
    let jobs = v["data"]["list"].as_array().unwrap();
    let job_by_code = |code: &str| {
        jobs.iter()
            .find(|j| j["code"] == code)
            .unwrap_or_else(|| panic!("缺任务 {code}: {v}"))
            .clone()
    };
    let retention = job_by_code("log_retention");
    let avatar_job = job_by_code("avatar_orphan_clean");
    assert_eq!(retention["name"], "日志清理", "{v}");
    assert_eq!(retention["cron"], "86400", "默认间隔 24 小时: {v}");
    assert_eq!(retention["status"], 1, "默认启用: {v}");
    let retention_id = common::as_id(&retention["id"]);
    let avatar_id = common::as_id(&avatar_job["id"]);

    // 筛选
    let (_st, v) = call(&c, Method::GET, api("/jobs?name=日志"), Some(&admin), None).await;
    assert_eq!(v["data"]["total"], 1, "name 模糊筛选: {v}");
    let (_st, v) = call(&c, Method::GET, api("/jobs?status=0"), Some(&admin), None).await;
    assert_eq!(v["data"]["total"], 0, "status 筛选: {v}");

    // ── 手动触发：同步执行 + 写 job_log + 更新 job 行 ───────────
    // 启动第一个 tick 就会把没跑过的任务跑一遍，所以只能比增量，不能比绝对值
    let before = job_log_count(&pool, "avatar_orphan_clean").await;
    let (st, v) = call(&c, Method::POST, api(&format!("/jobs/{avatar_id}/run")), Some(&admin), None).await;
    assert_eq!(st, 200, "手动触发: {v}");
    assert_eq!(v["data"]["status"], 1, "执行成功: {v}");
    assert!(v["data"]["msg"].as_str().unwrap_or("").contains("头像"), "返回执行摘要: {v}");
    assert!(v["data"]["duration_ms"].is_u64(), "返回耗时: {v}");
    let avatar_runs = job_log_count(&pool, "avatar_orphan_clean").await;
    assert_eq!(avatar_runs, before + 1, "手动触发写一条 job_log（启动 tick 不算）: {before} → {avatar_runs}");
    let last: Option<String> =
        sqlx::query_scalar("SELECT last_msg FROM job WHERE code = 'avatar_orphan_clean'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(last.as_deref().unwrap_or("").contains("头像"), "job.last_msg 要留痕: {last:?}");

    // 库里残留的旧 code：不可手动触发（409），避免点到已下线的任务
    let ghost = common::new_row_id();
    sqlx::query(
        "INSERT INTO job (id, name, code, cron, status, last_msg, created_at, updated_at) \
         VALUES (?, '幽灵任务', 'ghost_job', '60', 1, '', NOW(), NOW())",
    )
    .bind(ghost)
    .execute(&pool)
    .await
    .unwrap();
    // 库里的 id 是数字，URL 收 hashid：测试侧 encode 一下（后端返回的串才直接透传）
    let (st, v) = call(&c, Method::POST, api(&format!("/jobs/{}/run", common::enc_id(ghost as u64))), Some(&admin), None).await;
    assert_eq!(st, 409, "未注册 code 必须 409: {v}");
    assert!(v["msg"].as_str().unwrap_or("").contains("未在代码中注册"), "{v}");
    assert_eq!(v["err"], "job.not_registered", "{v}");
    assert_eq!(v["args"]["code"], "ghost_job", "args.code 是 code 原始值: {v}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_log WHERE job_code = 'ghost_job'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "409 的任务不能留下执行记录");

    // 不存在的任务 / 非法间隔
    let nope = common::enc_id(999_999); // 合法 hashid、库里没有
    let (st, v) = call(&c, Method::POST, api(&format!("/jobs/{nope}/run")), Some(&admin), None).await;
    assert_eq!(st, 404, "不存在: {v}");
    let (st, v) = call(&c, Method::PUT, api(&format!("/jobs/{nope}")), Some(&admin), Some(json!({"cron": "60", "status": 1}))).await;
    assert_eq!(st, 404, "更新不存在: {v}");
    for bad in ["abc", "", "0", "-5", "3.5"] {
        let (st, v) = call(&c, Method::PUT, api(&format!("/jobs/{retention_id}")), Some(&admin),
            Some(json!({"cron": bad, "status": 1}))).await;
        assert_eq!(st, 400, "间隔 {bad:?} 必须 400: {v}");
    }

    // ── 调度循环真跑：把间隔改成 2 秒，等它自己跑出新的执行记录 ──
    let before = job_log_count(&pool, "log_retention").await;
    let (st, v) = call(&c, Method::PUT, api(&format!("/jobs/{retention_id}")), Some(&admin),
        Some(json!({"cron": "2", "status": 1}))).await;
    assert_eq!(st, 200, "改间隔为 2 秒: {v}");
    let (_st, v) = call(&c, Method::GET, api("/jobs"), Some(&admin), None).await;
    let j = v["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["id"] == json!(retention_id))
        .unwrap();
    assert_eq!(j["cron"], "2", "间隔已落库: {v}");
    assert_eq!(j["name"], "日志清理", "名字不可改（更新体里没有这个字段）: {v}");

    // tick 是 10 秒一次，给足余量；轮询到出现新记录为止
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    let mut after = before;
    while after <= before && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        after = job_log_count(&pool, "log_retention").await;
    }
    assert!(after > before, "调度循环没跑：等 40 秒后 job_log 仍只有 {before} 条");
    let (st, v) = call(&c, Method::GET, api("/jobs"), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    let j = v["data"]["list"].as_array().unwrap().iter().find(|j| j["id"] == json!(retention_id)).unwrap();
    assert_eq!(j["last_status"], 1, "调度跑的也写状态: {j}");
    assert!(j["last_run_at"].is_string(), "调度跑的也写 last_run_at: {j}");
    // 跑过一次就够了，把间隔还原，别让后面几十秒一直往 job_log 里写
    let (st, _) = call(&c, Method::PUT, api(&format!("/jobs/{retention_id}")), Some(&admin),
        Some(json!({"cron": "86400", "status": 1}))).await;
    assert_eq!(st, 200);

    // ── job-logs 查询与筛选 ────────────────────────────────────
    let (st, v) = call(&c, Method::GET, api("/job-logs"), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    let total_all = v["data"]["total"].as_i64().unwrap();
    assert!(total_all >= 2, "手动 + 调度至少两条: {v}");
    let (st, v) = call(&c, Method::GET, api("/job-logs?job_code=avatar_orphan_clean"), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], avatar_runs, "按 code 筛选: {v}");
    assert!(
        v["data"]["list"].as_array().unwrap().iter().all(|r| r["job_code"] == "avatar_orphan_clean"),
        "筛选不能漏别的任务进来: {v}"
    );
    assert!(v["data"]["list"][0]["duration_ms"].is_u64(), "耗时要落库: {v}");
    let (st, v) = call(&c, Method::GET, api("/job-logs?status=0"), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 0, "没有失败记录: {v}");
    let (st, v) = call(&c, Method::GET, api("/job-logs?start=2000-01-01%2000:00:00&end=2000-01-02%2000:00:00"), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 0, "时间范围筛选: {v}");

    // ── 头像孤儿清理的行为 ─────────────────────────────────────
    let dir = std::path::Path::new("target/test-uploads/avatar");
    std::fs::create_dir_all(dir).unwrap();
    let admin_id = sqlx::query_scalar::<_, i64>("SELECT id FROM admin WHERE username = 'admin'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let admin_avatar = format!("{admin_id}.png");
    for name in [
        "999999.png".to_string(),  // 纯数字但 admin 不存在 → 删
        admin_avatar.clone(),      // 纯数字且 admin 在 → 留
        "notes.txt".to_string(),   // 非数字名 → 留
        "12.png.bak".to_string(),  // stem 带扩展，不是纯数字 → 留
        "头像.png".to_string(),    // 非数字名 → 留
    ] {
        std::fs::write(dir.join(&name), b"x").unwrap();
    }
    let (st, v) = call(&c, Method::POST, api(&format!("/jobs/{avatar_id}/run")), Some(&admin), None).await;
    assert_eq!(st, 200, "跑头像清理: {v}");
    assert!(
        v["data"]["msg"].as_str().unwrap_or("").contains("删除孤儿 1 个"),
        "只删那一个孤儿: {v}"
    );
    assert!(!dir.join("999999.png").exists(), "孤儿文件必须删掉");
    for kept in [
        admin_avatar.clone(),
        "notes.txt".to_string(),
        "12.png.bak".to_string(),
        "头像.png".to_string(),
    ] {
        assert!(dir.join(&kept).exists(), "{kept} 不能碰");
    }
    std::fs::remove_file(dir.join(&admin_avatar)).ok();

    // ── 公告：管理侧 CRUD + 发布语义 ───────────────────────────
    let (st, v) = call(&c, Method::GET, api("/notices"), Some(&admin), None).await;
    assert_eq!(st, 200, "空列表: {v}");
    assert_eq!(v["data"]["total"], 0, "{v}");

    let (st, v) = call(&c, Method::POST, api("/notices"), Some(&admin), Some(json!({
        "title": "系统维护", "content": "今晚 22:00 停机维护", "status": 1,
    }))).await;
    assert_eq!(st, 200, "发公告: {v}");
    let n1 = common::as_id(&v["data"]["id"]);
    let n1_published: chrono::NaiveDateTime = sqlx::query_scalar("SELECT published_at FROM notice WHERE id = ?")
        .bind(common::dec_id(&n1))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(n1_published.to_string().len() > 0, "发布即写 published_at: {n1_published}");

    let (st, v) = call(&c, Method::POST, api("/notices"), Some(&admin), Some(json!({
        "title": "草稿箱", "content": "还没想好", "status": 0,
    }))).await;
    assert_eq!(st, 200, "存草稿: {v}");
    let n2 = common::as_id(&v["data"]["id"]);
    let n2_published: Option<chrono::NaiveDateTime> = sqlx::query_scalar("SELECT published_at FROM notice WHERE id = ?")
        .bind(common::dec_id(&n2))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(n2_published.is_none(), "草稿不写 published_at");

    let (_st, v) = call(&c, Method::GET, api("/notices?status=0"), Some(&admin), None).await;
    assert_eq!(v["data"]["total"], 1, "status 筛选: {v}");
    assert_eq!(v["data"]["list"][0]["title"], "草稿箱", "{v}");
    // 列表行必须带 content：没有单条详情接口，前端编辑弹窗靠它回填，缺了就只能把正文清空
    assert_eq!(v["data"]["list"][0]["content"], "还没想好", "列表行要带 content（编辑回填用）: {v}");
    let (_st, v) = call(&c, Method::GET, api("/notices?title=维护"), Some(&admin), None).await;
    assert_eq!(v["data"]["total"], 1, "title 模糊筛选: {v}");
    for bad in ["", "  ", &"x".repeat(129)] {
        let (st, v) = call(&c, Method::POST, api("/notices"), Some(&admin),
            Some(json!({"title": bad, "content": "x", "status": 1}))).await;
        assert_eq!(st, 400, "标题 {bad:?} 必须 400: {v}");
    }
    // 正文同理：空正文 400（前端回填失败时会以 400 炸出来，不会静默把正文改空）
    for bad in ["", "  "] {
        let (st, v) = call(&c, Method::PUT, api(&format!("/notices/{n1}")), Some(&admin),
            Some(json!({"title": "系统维护", "content": bad, "status": 1}))).await;
        assert_eq!(st, 400, "空正文 {bad:?} 必须 400: {v}");
    }

    // 普通用户：没挂任何权限码
    let (st, v) = call(&c, Method::POST, api("/roles"), Some(&admin), Some(json!({
        "name": "v16_none", "code": "v16_none", "data_scope": 4, "status": 1,
    }))).await;
    assert_eq!(st, 200, "建空角色: {v}");
    let empty_role = common::as_id(&v["data"]["id"]);
    let (st, v) = call(&c, Method::POST, api("/admins"), Some(&admin), Some(json!({
        "username": "v16user", "password": "v16user123", "role_ids": [empty_role],
    }))).await;
    assert_eq!(st, 200, "建普通用户: {v}");
    let plain = common::login(&base, "v16user", "v16user123").await.expect("普通用户登录");

    // 未读：只含已发布，草稿完全不可见
    let (st, v) = call(&c, Method::GET, api("/notices/unread"), Some(&plain), None).await;
    assert_eq!(st, 200, "登录即可读未读（不要权限码）: {v}");
    assert_eq!(v["data"]["total"], 1, "草稿不进未读: {v}");
    let unread = v["data"]["list"].as_array().unwrap();
    assert_eq!(unread.len(), 1, "{v}");
    assert_eq!(unread[0]["id"], json!(n1), "{v}");
    assert_eq!(unread[0]["title"], "系统维护", "{v}");

    // 未登录 → 401（不是 403）
    let (st, v) = call(&c, Method::GET, api("/notices/unread"), None, None).await;
    assert_eq!(st, 401, "未登录: {v}");
    let (st, v) = call(&c, Method::POST, api(&format!("/notices/{n1}/read")), None, None).await;
    assert_eq!(st, 401, "未登录: {v}");

    // 标记已读：幂等；再次标记仍 200
    let (st, v) = call(&c, Method::POST, api(&format!("/notices/{n1}/read")), Some(&plain), None).await;
    assert_eq!(st, 200, "标记已读: {v}");
    let (st, v) = call(&c, Method::POST, api(&format!("/notices/{n1}/read")), Some(&plain), None).await;
    assert_eq!(st, 200, "重复标记仍 200（幂等）: {v}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notice_read WHERE notice_id = ? AND admin_id <> 0")
        .bind(common::dec_id(&n1))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "幂等：只留一行已读记录");
    let (st, v) = call(&c, Method::GET, api("/notices/unread"), Some(&plain), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 0, "读过的从列表消失: {v}");
    assert_eq!(v["data"]["list"], json!([]), "{v}");

    // 草稿 / 不存在：标记已读一律 404（草稿不属于任何人可见的未读集合）
    let (st, v) = call(&c, Method::POST, api(&format!("/notices/{n2}/read")), Some(&plain), None).await;
    assert_eq!(st, 404, "标记草稿为已读: {v}");
    let (st, v) = call(&c, Method::POST, api(&format!("/notices/{nope}/read")), Some(&plain), None).await;
    assert_eq!(st, 404, "标记不存在的公告: {v}");

    // 管理接口对普通用户全 403
    for (m, path, body) in [
        (Method::GET, "/notices".to_string(), None),
        (Method::POST, "/notices".to_string(), Some(json!({"title": "x", "content": "x", "status": 1}))),
        (Method::PUT, format!("/notices/{n1}"), Some(json!({"title": "x", "content": "x", "status": 1}))),
        (Method::DELETE, format!("/notices/{n1}"), None),
        (Method::GET, "/jobs".to_string(), None),
        (Method::PUT, format!("/jobs/{retention_id}"), Some(json!({"cron": "60", "status": 1}))),
        (Method::POST, format!("/jobs/{retention_id}/run"), None),
        (Method::GET, "/job-logs".to_string(), None),
    ] {
        let (st, v) = call(&c, m.clone(), api(&path), Some(&plain), body).await;
        assert_eq!(st, 403, "{m} {path} 无权限必须 403: {v}");
        assert!(v["msg"].as_str().unwrap_or("").starts_with("缺少权限"), "{m} {path}: {v}");
        assert_eq!(v["err"], "auth.forbidden", "{m} {path} 403 要带码: {v}");
        assert!(v["args"]["code"].as_str().unwrap_or("").starts_with("system:"),
            "args.code 给出缺的权限码: {m} {path}: {v}");
    }

    // ── published_at 只在首次发布时写（改回草稿不清空）───────────
    let (st, v) = call(&c, Method::PUT, api(&format!("/notices/{n2}")), Some(&admin), Some(json!({
        "title": "草稿箱", "content": "想好了", "status": 1,
    }))).await;
    assert_eq!(st, 200, "发布草稿: {v}");
    let first_publish: chrono::NaiveDateTime = sqlx::query_scalar("SELECT published_at FROM notice WHERE id = ?")
        .bind(common::dec_id(&n2))
        .fetch_one(&pool)
        .await
        .unwrap();
    // 刚发布 = 时间就在当下（DATETIME 无小数秒且 MySQL 按四舍五入进位，允许差 1 秒）
    let skew = (first_publish - chrono::Local::now().naive_local()).num_seconds();
    assert!(skew.abs() <= 1, "首次发布要写当下的时间，实际 {first_publish}");
    // 等过一秒：如果实现是「每次置 1 都重写」，时间串就会变，断言才测得出来
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let (st, v) = call(&c, Method::PUT, api(&format!("/notices/{n2}")), Some(&admin), Some(json!({
        "title": "草稿箱", "content": "想好了", "status": 0,
    }))).await;
    assert_eq!(st, 200, "撤回: {v}");
    let withdrawn: Option<chrono::NaiveDateTime> = sqlx::query_scalar("SELECT published_at FROM notice WHERE id = ?")
        .bind(common::dec_id(&n2))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(withdrawn, Some(first_publish), "撤回不清 published_at（留痕）");
    let (st, v) = call(&c, Method::PUT, api(&format!("/notices/{n2}")), Some(&admin), Some(json!({
        "title": "草稿箱", "content": "想好了", "status": 1,
    }))).await;
    assert_eq!(st, 200, "再发布: {v}");
    let republished: chrono::NaiveDateTime = sqlx::query_scalar("SELECT published_at FROM notice WHERE id = ?")
        .bind(common::dec_id(&n2))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(republished, first_publish, "published_at 只在首次发布时写，重发不改写");

    // 撤回期间（草稿）也不该进未读
    let (st, v) = call(&c, Method::GET, api("/notices/unread"), Some(&plain), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 1, "只有 n2 未读: {v}");
    assert_eq!(v["data"]["list"][0]["id"], json!(n2), "{v}");

    // ── 未读分页：total 是未读总数，不是 list.length ─────────────
    for i in 0..52 {
        sqlx::query(
            "INSERT INTO notice (id, title, content, status, created_by, published_at, created_at, updated_at) \
             VALUES (?, ?, '批量公告', 1, ?, NOW(), NOW(), NOW())",
        )
        .bind(common::new_row_id())
        .bind(format!("批量 {i}"))
        .bind(admin_id) // 超管的库内数字 id（不再是 1）
        .execute(&pool)
        .await
        .unwrap();
    }
    let (st, v) = call(&c, Method::GET, api("/notices/unread"), Some(&plain), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 53, "未读总数（52 条批量 + n2）: {v}");
    let list = v["data"]["list"].as_array().unwrap();
    assert_eq!(list.len(), 50, "列表只回最近 50 条: {v}");
    // 排序 id DESC，且摘掉一条后从列表消失。
    // 对外是 hashid 串，比不了大小：解码回数字再比（URL 里仍用原串透传）
    let ids: Vec<i64> = list.iter().map(|n| common::dec_id(n["id"].as_str().unwrap())).collect();
    assert!(ids.windows(2).all(|w| w[0] > w[1]), "按 id DESC: {ids:?}");
    let first_hash = common::as_id(&list[0]["id"]);
    let first_id = ids[0];
    let (st, v) = call(&c, Method::POST, api(&format!("/notices/{first_hash}/read")), Some(&plain), None).await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = call(&c, Method::GET, api("/notices/unread"), Some(&plain), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 52, "读一条少一条: {v}");
    let ids: Vec<i64> = v["data"]["list"].as_array().unwrap().iter()
        .map(|n| common::dec_id(n["id"].as_str().unwrap())).collect();
    assert!(!ids.contains(&first_id), "读过的必须从列表消失: {ids:?}");

    // ── 删除：同一事务里清 notice_read ──────────────────────────
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notice_read WHERE notice_id = ?")
        .bind(common::dec_id(&n1))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, 1, "n1 有一条已读记录（上面标的）");
    let (st, v) = call(&c, Method::DELETE, api(&format!("/notices/{n1}")), Some(&admin), None).await;
    assert_eq!(st, 200, "删公告: {v}");
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notice_read WHERE notice_id = ?")
        .bind(common::dec_id(&n1))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after, 0, "级联删已读记录，不留垃圾行");
    let (st, v) = call(&c, Method::DELETE, api(&format!("/notices/{n1}")), Some(&admin), None).await;
    assert_eq!(st, 404, "重复删 404: {v}");
    let (st, v) = call(&c, Method::DELETE, api(&format!("/notices/{nope}")), Some(&admin), None).await;
    assert_eq!(st, 404, "删不存在: {v}");

    // 删公告的动作要进审计（module=notice，动作有中文名）
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE module = 'notice' AND action = '删除公告'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    // 4 = 普通用户被 403 拒 + 删除成功 + 重复删 404 + 删不存在的 404（失败写操作也留痕）
    assert_eq!(n, 4, "删公告全部留痕: {n}");
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE module = 'job' AND action = '手动执行定时任务'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    // 5 = 删不存在的任务 404 + 幽灵任务 409 + 超管手动跑成功 + 普通用户 403 + 头像清理那次成功
    assert_eq!(n, 5, "手动触发全部留痕: {n}");

    // 重开进程：菜单与任务补齐都是幂等的
    drop(_server);
    let (_server2, base2) = common::start_server().await;
    let admin2 = common::login(&base2, "admin", "admin123").await.expect("重启后登录");
    let (st, v) = call(&c, Method::GET, format!("{base2}/api/v1/menus/tree"), Some(&admin2), None).await;
    assert_eq!(st, 200, "{v}");
    assert!(menu(&v["data"], "system:job:list").is_some(), "重启后菜单还在: {v}");
    let (st, v) = call(&c, Method::GET, format!("{base2}/api/v1/jobs"), Some(&admin2), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 3, "补的是缺的：库里原有的 ghost_job 不被删（也不重复补内置的）: {v}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job WHERE code = 'log_retention'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "内置任务只有一条: {n}");
    drop(_server2);

    // ── [job] enabled=false：不启动循环，但手动触发仍可用 ─────────
    common::reset_db(&dsn).await;
    let (_server3, base3) = start_server_conf("[job]\nenabled = false").await;
    let admin3 = common::login(&base3, "admin", "admin123").await.expect("关定时后登录");
    let api3 = |p: &str| format!("{base3}/api/v1{p}");
    let (st, v) = call(&c, Method::GET, api3("/jobs"), Some(&admin3), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 2, "补齐不受开关影响（运维要能看到任务）: {v}");
    let rid = common::as_id(
        &v["data"]["list"].as_array().unwrap().iter()
            .find(|j| j["code"] == "log_retention").unwrap()["id"],
    );
    // 新库 last_run_at 全空 = 一旦循环启动，第一个 tick 就会跑；等 5 秒断言没跑
    tokio::time::sleep(Duration::from_secs(5)).await;
    let n = job_log_count(&pool, "log_retention").await;
    assert_eq!(n, 0, "[job] enabled=false 时循环不该启动，实际跑了 {n} 次");
    let last: Option<chrono::NaiveDateTime> = sqlx::query_scalar("SELECT last_run_at FROM job WHERE code = 'log_retention'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(last.is_none(), "循环没启动，last_run_at 必须还是空: {last:?}");

    // 手动触发不看总开关（运维要能临时关掉定时、手动补跑）
    let (st, v) = call(&c, Method::POST, api3(&format!("/jobs/{rid}/run")), Some(&admin3), None).await;
    assert_eq!(st, 200, "关定时后手动触发仍要可用: {v}");
    assert_eq!(v["data"]["status"], 1, "{v}");
    let n = job_log_count(&pool, "log_retention").await;
    assert_eq!(n, 1, "手动跑一次写一条: {n}");
    // 再等一个 tick 的时间：手动跑过也不会让循环松口
    tokio::time::sleep(Duration::from_secs(3)).await;
    let n = job_log_count(&pool, "log_retention").await;
    assert_eq!(n, 1, "循环仍然没启动: {n}");
}
