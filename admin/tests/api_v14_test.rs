// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! BRA v1.4 后端新能力集成测试：A1 路由级 404/405 信封 / A2 登录时序侧信道（行为面）/
//! A3 退出其他设备 / A4 日志保留策略 / B6 流式 CSV 导出。
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

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

/// 菜单树里按权限码取菜单 id（B5 造角色要勾菜单）；对外 id 是 hashid 串，原样透传。
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

/// 建角色 + 勾菜单，返回角色 id（B5 造场景用）。
async fn make_role(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    scope: i8,
    status: i8,
    menu_ids: &[String],
) -> String {
    let (st, v) = call(
        c,
        Method::POST,
        format!("{base}/api/v1/roles"),
        Some(token),
        Some(json!({"name": name, "code": name, "data_scope": scope, "status": status})),
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

#[tokio::test]
async fn v14_backend_features() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;

    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── A1 路由级 404/405 也走统一信封 ──────────────────────
    let r = c.get(api("/nope/none")).send().await.unwrap();
    assert_eq!(r.status(), 404);
    let ct = r.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ct.starts_with("application/json"), "未匹配路径必须是 JSON 信封，实际 {ct}");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["code"], 404, "{v}");
    assert_eq!(v["msg"], "接口不存在", "{v}");
    assert_eq!(v["err"], "common.not_found", "路由级 404 也带码: {v}");
    assert!(v["data"].is_null(), "信封形状一致: {v}");

    // /api/v1/health 只注册了 GET
    let r = c.post(api("/health")).send().await.unwrap();
    assert_eq!(r.status(), 405);
    assert_eq!(r.headers()["content-type"], "application/json", "405 也必须是信封");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["code"], 405, "{v}");
    assert_eq!(v["msg"], "方法不允许", "{v}");

    // 树外的路径不受兜底影响（保持 axum 原样：空 body、无 content-type，不替别的服务改响应形状）
    let r = c.get(format!("{base}/nope")).send().await.unwrap();
    assert_eq!(r.status(), 404);
    let ct = r
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    assert!(!ct.contains("json"), "树外保持 axum 原样，实际 {ct}");
    assert!(r.bytes().await.unwrap().is_empty(), "树外 404 是空 body，不是信封");

    // 未匹配路径上的写操作仍被审计留痕（兜底挂在审计中间件之内）
    let (st, _v) = call(&c, Method::POST, api("/nope/none"), None, Some(json!({}))).await;
    assert_eq!(st, 404, "未知路径的 POST 也回信封");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE path = '/api/v1/nope/none'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "探测性写操作也要留痕: {n}");

    // ── A2 用户不存在：提示与失败记录不变（时序本身集成测试断言不了）──
    let (st, v) = call(&c, Method::POST, api("/auth/login"), None,
        Some(json!({"username": "ghost_none", "password": "whatever123"}))).await;
    assert_eq!(st, 400, "{v}");
    assert_eq!(v["msg"], "用户名或密码错误", "与密码错误同一句提示，不泄露用户是否存在: {v}");
    assert_eq!(v["err"], "auth.bad_credentials", "{v}");
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM login_log WHERE username = 'ghost_none' AND msg = '用户不存在'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "不存在的用户也要留失败记录: {n}");

    // ── A3 退出其他设备 ────────────────────────────────────
    // 两次登录 = 两台设备（JWT 到秒，同一秒内两枚 token 可能一模一样，
    // 所以断言只看有效性，不比 token 串是否相同）
    let t1 = common::login(&base, "admin", "admin123").await.expect("设备 1 登录");
    let t2 = common::login(&base, "admin", "admin123").await.expect("设备 2 登录");
    let (st, v) = call(&c, Method::POST, api("/auth/logout-others"), Some(&t1), None).await;
    assert_eq!(st, 200, "退出其他设备: {v}");
    let fresh = v["data"]["token"].as_str().expect("必须回新 token（前端从 data.token 取）").to_string();
    assert!(v["data"]["expires_in"].as_i64().unwrap_or(0) > 0, "与 login 同形状: {v}");

    let (st, v) = call(&c, Method::GET, api("/auth/profile"), Some(&fresh), None).await;
    assert_eq!(st, 200, "当前设备换发的新 token 立即可用: {v}");
    assert_eq!(v["data"]["user"]["username"], "admin");

    let (st, _v) = call(&c, Method::GET, api("/auth/profile"), Some(&t2), None).await;
    assert_eq!(st, 401, "其他设备的 token 被踢下线");
    let (st, _v) = call(&c, Method::GET, api("/auth/profile"), Some(&t1), None).await;
    assert_eq!(st, 401, "本机旧 token 同样失效（token_version 已自增）");

    let admin = fresh; // 后面继续用当前设备换发的新 token

    // ── B6 流式导出：跨批不丢不重 ──────────────────────────
    // 塞 5001 行（> csv::BATCH = 5000）逼出第二批；导出条数必须与库里行数严格相等
    let mut values = Vec::with_capacity(5001);
    for i in 0..5001 {
        // id 不再是自增列，得自己给（见 common::new_row_id）
        values.push(format!("({},1,'bulk{i}','10.0.0.1','',1,'',NOW())", common::new_row_id()));
    }
    for chunk in values.chunks(500) {
        sqlx::query(&format!(
            "INSERT INTO login_log (id, admin_id, username, ip, user_agent, status, msg, created_at) VALUES {}",
            chunk.join(",")
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_log").fetch_one(&pool).await.unwrap();
    let (min_id, max_id): (i64, i64) =
        sqlx::query_as("SELECT MIN(id), MAX(id) FROM login_log").fetch_one(&pool).await.unwrap();

    let lines = csv_lines(&c, api("/login-logs/export"), &admin).await;
    assert_eq!(lines[0], "ID,用户名,IP,User-Agent,结果,详情,时间", "表头");
    assert_eq!(lines.len() as i64, total + 1, "表头 + 全部 {total} 行，跨批不丢不重");
    // 导出里的 ID 也是对外短串：库里的 MIN/MAX 要过一遍编码才能比对
    let (max_hid, min_hid) = (common::enc_id(max_id as u64), common::enc_id(min_id as u64));
    assert!(lines[1].starts_with(&format!("{max_hid},")), "id DESC：首行是最新的 {max_id}: {}", lines[1]);
    assert!(
        lines[lines.len() - 1].starts_with(&format!("{min_hid},")),
        "最老的一行在（最后一批没被漏掉）: {}",
        lines[lines.len() - 1]
    );

    // 筛选与 list 同源（同一个 filtered()）：导出带上 username 过滤，条数跟着变
    let lines = csv_lines(&c, api("/login-logs/export?username=bulk0"), &admin).await;
    assert_eq!(lines.len(), 2, "username=bulk0 只命中一行: {lines:?}");

    // 另两个导出同样走流式路径，附件头/表头不变
    for (path, head) in [("/admins/export", "ID,用户名,昵称"), ("/audit-logs/export", "ID,时间,用户")] {
        let lines = csv_lines(&c, api(path), &admin).await;
        assert!(lines[0].starts_with(head), "{path} 表头: {}", lines[0]);
    }

    // ── A4 日志保留策略（[log] retain_days 默认 90）──────────
    // 各插一条过期行（2000 年）；清理在进程启动时跑，重启一个进程来触发（不真等 24 小时）
    sqlx::query(
        "INSERT INTO login_log (id, admin_id, username, ip, user_agent, status, msg, created_at) \
         VALUES (?, 1, 'old-login', '1.1.1.1', '', 1, '', '2000-01-01 00:00:00')",
    )
    .bind(common::new_row_id())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO audit_log (id, admin_id, username, module, action, method, path, status, msg, duration_ms, ip, created_at) \
         VALUES (?, 1, 'admin', 'other', '老记录', 'POST', '/api/v1/x', 0, '', 1, '1.1.1.1', '2000-01-01 00:00:00')",
    )
    .bind(common::new_row_id())
    .execute(&pool)
    .await
    .unwrap();

    let (_server2, base2) = common::start_server().await;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let old_login: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM login_log WHERE created_at < '2000-01-02'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let old_audit: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE created_at < '2000-01-02'")
                .fetch_one(&pool)
                .await
                .unwrap();
        if old_login == 0 && old_audit == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "启动清理没在 20 秒内删掉过期日志（login_log {old_login} / audit_log {old_audit}）"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // 保留窗口内的行一条都不能少（上面刚塞的 5001 行还在）
    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_log").fetch_one(&pool).await.unwrap();
    assert_eq!(kept, total, "窗口内的日志必须原样保留: {kept} != {total}");

    // 触发清理的那个进程也要是能用的
    let r = c.get(format!("{base2}/api/v1/health")).send().await.unwrap();
    assert_eq!(r.status(), 200);

    // ── B5 非列表接口的数据权限 + 反自我提权 ─────────────────
    // 场景：操作者 b5op 是 scope=3（本部门 A）；目标 t1 在 A 部、t2 在 B 部
    let (st, v) = call(&c, Method::POST, api("/depts"), Some(&admin), Some(json!({"name": "B5-A部"}))).await;
    assert_eq!(st, 200, "建部门 A: {v}");
    let dept_a = common::as_id(&v["data"]["id"]);
    let (st, v) = call(&c, Method::POST, api("/depts"), Some(&admin), Some(json!({"name": "B5-B部"}))).await;
    assert_eq!(st, 200, "建部门 B: {v}");
    let dept_b = common::as_id(&v["data"]["id"]);

    let (st, v) = call(&c, Method::GET, api("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200);
    let tree = v["data"].clone();
    let perm_menu = |p: &str| menu_id(&tree, p).unwrap_or_else(|| panic!("种子菜单里没有 {p}"));

    // 操作者：管理员管理全套权限（list/add/edit/remove/resetPwd）+ 角色列表（分配角色
    // 时得看得见角色）+ scope=3
    let op_menus: Vec<String> = [
        "system:admin:list", "system:admin:add", "system:admin:edit",
        "system:admin:remove", "system:admin:resetPwd", "system:role:list",
    ].iter().map(|p| perm_menu(p)).collect();
    let op_role = make_role(&c, &base, &admin, "b5_scope3", 3, 1, &op_menus).await;
    // 更宽的角色（scope=1）：只有超管能授
    let wide_role = make_role(&c, &base, &admin, "b5_wide", 1, 1, &[]).await;
    let wide2_role = make_role(&c, &base, &admin, "b5_wide2", 1, 1, &[]).await;
    // 含操作者没有的权限码（system:role:edit）的角色
    let alien_role = make_role(&c, &base, &admin, "b5_alien", 4, 1, &[perm_menu("system:role:edit")]).await;
    // scope=4 且权限是操作者子集：可以授
    let ok_role = make_role(&c, &base, &admin, "b5_ok", 4, 1, &[perm_menu("system:admin:list")]).await;
    let off_role = make_role(&c, &base, &admin, "b5_off", 4, 0, &[]).await;

    let (st, v) = call(&c, Method::POST, api("/admins"), Some(&admin), Some(json!({
        "username": "b5op", "password": "b5op12345", "dept_id": dept_a, "role_ids": [op_role],
    }))).await;
    assert_eq!(st, 200, "建操作者: {v}");
    let op_id = common::as_id(&v["data"]["id"]);
    let (st, v) = call(&c, Method::POST, api("/admins"), Some(&admin), Some(json!({
        "username": "b5t1", "password": "b5t12345", "dept_id": dept_a,
    }))).await;
    assert_eq!(st, 200, "建 A 部目标: {v}");
    let t1 = common::as_id(&v["data"]["id"]);
    let (st, v) = call(&c, Method::POST, api("/admins"), Some(&admin), Some(json!({
        "username": "b5t2", "password": "b5t12345", "dept_id": dept_b,
    }))).await;
    assert_eq!(st, 200, "建 B 部目标: {v}");
    let t2 = common::as_id(&v["data"]["id"]);

    let op = common::login(&base, "b5op", "b5op12345").await.expect("操作者登录");

    // 范围内的目标照常可用
    let (st, v) = call(&c, Method::GET, api(&format!("/admins/{t1}")), Some(&op), None).await;
    assert_eq!(st, 200, "本部门详情: {v}");
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}")), Some(&op), Some(json!({
        "nickname": "本部门改的", "dept_id": dept_a, "status": 1, "role_ids": [],
    }))).await;
    assert_eq!(st, 200, "本部门编辑（不授角色）: {v}");

    // 范围外的目标：详情/编辑/改状态/改密码/删除/改角色，一个都不许碰
    // （编辑那条的 dept_id 故意填范围内的，让「目标在不在范围内」成为唯一拦截理由）
    for (m, path, body) in [
        (Method::GET, format!("/admins/{t2}"), None),
        (Method::PUT, format!("/admins/{t2}"),
            Some(json!({"nickname": "越界", "dept_id": dept_a, "status": 1, "role_ids": []}))),
        (Method::PUT, format!("/admins/{t2}/status"), Some(json!({"status": 0}))),
        (Method::PUT, format!("/admins/{t2}/password"), Some(json!({"password": "hacked12345"}))),
        (Method::DELETE, format!("/admins/{t2}"), None),
        (Method::PUT, format!("/admins/{t2}/roles"), Some(json!({"role_ids": [ok_role]}))),
    ] {
        let (st, v) = call(&c, m.clone(), api(&path), Some(&op), body).await;
        assert_eq!(st, 403, "{m} {path} 跨部门必须 403: {v}");
        assert_eq!(v["msg"], "超出你的数据权限范围", "{m} {path}: {v}");
        assert_eq!(v["err"], "scope.out_of_range", "稳定码要能对上表: {v}");
    }
    // 范围内的人也不能挪到范围外的部门去（否则能绕开上面的目标判定）
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}")), Some(&op), Some(json!({
        "nickname": "本部门改的", "dept_id": dept_b, "status": 1, "role_ids": [],
    }))).await;
    assert_eq!(st, 403, "不能把人挪到范围外的部门: {v}");
    assert_eq!(v["msg"], "超出你的数据权限范围", "{v}");
    assert_eq!(v["err"], "scope.out_of_range", "{v}");

    // 反自我提权：给自己授 data_scope=1 → 403（修复前会成功，这就是提权路径）
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{op_id}/roles")), Some(&op),
        Some(json!({"role_ids": [wide_role]}))).await;
    assert_eq!(st, 403, "给自己授 data_scope=1 就是自我提权: {v}");
    assert_eq!(v["msg"], "不能授予数据范围更宽的角色", "{v}");
    assert_eq!(v["err"], "scope.wider_role", "{v}");
    // 授给本部门的人一样拦：等于给自己发个马甲
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}/roles")), Some(&op),
        Some(json!({"role_ids": [op_role, wide_role]}))).await;
    assert_eq!(st, 403, "同部门的人也不能授更宽的角色: {v}");

    // 授自己没有的权限码 → 403
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}/roles")), Some(&op),
        Some(json!({"role_ids": [alien_role]}))).await;
    assert_eq!(st, 403, "授自己没有的权限码: {v}");
    assert_eq!(v["msg"], "不能授予包含你没有的权限的角色", "{v}");
    assert_eq!(v["err"], "scope.extra_perms", "{v}");

    // 子集 + scope=4 放行；清空角色不是提权，也放行
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}/roles")), Some(&op),
        Some(json!({"role_ids": [ok_role]}))).await;
    assert_eq!(st, 200, "scope=4 且权限是子集应放行: {v}");
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}/roles")), Some(&op),
        Some(json!({"role_ids": []}))).await;
    assert_eq!(st, 200, "清空角色放行: {v}");

    // 存在性 + 停用（原先不校验，写进去就是脏关系）
    // 999_999 → 测试侧 encode 成合法 hashid：解码没问题，但库里没这个角色
    for bad in [off_role.clone(), common::enc_id(999_999)] {
        let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}/roles")), Some(&op),
            Some(json!({"role_ids": [bad]}))).await;
        assert_eq!(st, 400, "角色 {bad} 不存在或已停用: {v}");
        assert_eq!(v["msg"], "角色不存在或已停用", "{v}");
        assert_eq!(v["err"], "scope.role_unavailable", "{v}");
    }

    // 角色列表/详情带 grantable：前端照它灰掉选项，别摆出「点了才被骂」的角色
    let (st, v) = call(&c, Method::GET, api("/roles?size=100"), Some(&op), None).await;
    assert_eq!(st, 200, "操作者读角色列表: {v}");
    let grant = |id: &str| {
        v["data"]["list"].as_array().unwrap().iter()
            .find(|r| r["id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("角色 {id} 不在列表里: {v}"))["grantable"]
            .as_bool()
            .unwrap_or(false)
    };
    assert!(!grant(&wide_role), "data_scope=1 不该标记为可授: {v}");
    assert!(!grant(&alien_role), "权限不是自己子集的不可授: {v}");
    assert!(!grant(&off_role), "已停用的不可授: {v}");
    assert!(grant(&op_role), "自己那类角色可授: {v}");
    assert!(grant(&ok_role), "scope=4 且权限是子集可授: {v}");

    let (st, vd) = call(&c, Method::GET, api(&format!("/roles/{wide_role}")), Some(&op), None).await;
    assert_eq!(st, 200, "角色详情: {vd}");
    assert_eq!(vd["data"]["grantable"], false, "详情同样带标记: {vd}");

    // 保留已有的不算授予：超管给 t1 授过 scope=1 的角色之后，操作者改 t1 的昵称
    // （编辑表单会把现有 role_ids 一起提交）不能被整个拒掉
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}/roles")), Some(&admin),
        Some(json!({"role_ids": [wide_role]}))).await;
    assert_eq!(st, 200, "超管先给 t1 授一个 scope=1 的角色: {v}");
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}")), Some(&op), Some(json!({
        "nickname": "改个昵称", "dept_id": dept_a, "status": 1, "role_ids": [wide_role],
    }))).await;
    assert_eq!(st, 200, "原样带上已有的宽角色改昵称必须放行: {v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/admins/{t1}")), Some(&op), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["nickname"], "改个昵称", "昵称确实改了: {v}");

    // 差集只放过「已有」的那部分：新增宽角色照样 403
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}/roles")), Some(&op),
        Some(json!({"role_ids": [wide_role, wide2_role]}))).await;
    assert_eq!(st, 403, "新增的宽角色仍要 403（原有保护不能被削掉）: {v}");
    assert_eq!(v["msg"], "不能授予数据范围更宽的角色", "{v}");
    assert_eq!(v["err"], "scope.wider_role", "{v}");
    // 移除已有角色不算越权：收窄权限不需要谁的许可
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t1}/roles")), Some(&op),
        Some(json!({"role_ids": []}))).await;
    assert_eq!(st, 200, "移除已有角色放行: {v}");

    // 建人：部门与角色同样受限
    let (st, v) = call(&c, Method::POST, api("/admins"), Some(&op), Some(json!({
        "username": "b5new", "password": "b5new12345", "dept_id": dept_b,
    }))).await;
    assert_eq!(st, 403, "不能在范围外的部门建人: {v}");
    let (st, v) = call(&c, Method::POST, api("/admins"), Some(&op), Some(json!({
        "username": "b5new", "password": "b5new12345", "dept_id": dept_a, "role_ids": [wide_role],
    }))).await;
    assert_eq!(st, 403, "建人不能顺手授更宽的角色: {v}");
    let (st, v) = call(&c, Method::POST, api("/admins"), Some(&op), Some(json!({
        "username": "b5new", "password": "b5new12345", "dept_id": dept_a, "role_ids": [ok_role],
    }))).await;
    assert_eq!(st, 200, "本部门建人 + 子集角色: {v}");

    // 超管全部放行
    let (st, v) = call(&c, Method::GET, api(&format!("/admins/{t2}")), Some(&admin), None).await;
    assert_eq!(st, 200, "超管看 B 部的人: {v}");
    let (st, v) = call(&c, Method::PUT, api(&format!("/admins/{t2}")), Some(&admin), Some(json!({
        "nickname": "超管改的", "dept_id": dept_b, "status": 1, "role_ids": [wide_role],
    }))).await;
    assert_eq!(st, 200, "超管改 B 部的人并授 scope=1: {v}");

    // 超管的角色列表：全部 grantable（显示口径与写路径一致）
    let (st, v) = call(&c, Method::GET, api("/roles?size=100"), Some(&admin), None).await;
    assert_eq!(st, 200, "超管读角色列表: {v}");
    let rows = v["data"]["list"].as_array().unwrap();
    assert!(rows.len() >= 5, "至少 5 个角色（含上面停用的那个）: {v}");
    assert!(rows.iter().all(|r| r["grantable"] == true), "超管全可授: {v}");
}
