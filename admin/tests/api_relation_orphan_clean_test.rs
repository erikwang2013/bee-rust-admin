// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 删除角色/部门/菜单是「先查引用再删」，与并发授权之间有窗口（#8）。
//! 日志保留任务每轮兜底清 `admin_role` / `role_menu` / `role_dept` 的悬挂关联，
//! 并把指向已删部门的 `admin.dept_id` 归零；本文件验证只清孤儿、不动有效关系。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

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
    (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
}

fn find_perm<'a>(menus: &'a Value, perm: &str) -> Option<&'a str> {
    for m in menus.as_array().into_iter().flatten() {
        if m["perm"].as_str() == Some(perm) {
            return m["id"].as_str();
        }
        if let Some(id) = find_perm(&m["children"], perm) {
            return Some(id);
        }
    }
    None
}

#[tokio::test]
async fn log_retention_cleans_orphan_relations() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let c = reqwest::Client::new();
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");

    // ── 造一组**有效**关系：角色-菜单、角色-部门、用户-角色、用户-部门 ──
    let (st, v) = call(&c, Method::GET, api("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200, "菜单树: {v}");
    let menu = find_perm(&v["data"], "system:notice:list").expect("种子里有 system:notice:list");
    let (st, v) =
        call(&c, Method::POST, api("/depts"), Some(&admin), Some(json!({"name": "清理测试部"})))
            .await;
    assert_eq!(st, 200, "建部门: {v}");
    let dept = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::POST,
        api("/roles"),
        Some(&admin),
        Some(json!({"name": "清理角色", "code": "orphan_clean_role", "data_scope": 5, "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "建角色: {v}");
    let role = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/roles/{role}/menus")),
        Some(&admin),
        Some(json!({"menu_ids": [menu]})),
    )
    .await;
    assert_eq!(st, 200, "角色挂菜单: {v}");
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/roles/{role}/depts")),
        Some(&admin),
        Some(json!({"dept_ids": [dept]})),
    )
    .await;
    assert_eq!(st, 200, "角色挂部门: {v}");
    for (name, with_dept) in [("orphan_a", true), ("orphan_b", false)] {
        let mut body = json!({"username": name, "password": "pass12345", "role_ids": [role]});
        if with_dept {
            body["dept_id"] = json!(dept);
        }
        let (st, v) = call(&c, Method::POST, api("/admins"), Some(&admin), Some(body)).await;
        assert_eq!(st, 200, "建用户 {name}: {v}");
    }
    let id_of = |username: &str| {
        let pool = pool.clone();
        let username = username.to_string();
        async move {
            let id: i64 = sqlx::query_scalar("SELECT id FROM admin WHERE username = ?")
                .bind(username)
                .fetch_one(&pool)
                .await
                .unwrap();
            id
        }
    };
    let (admin_a, admin_b) = (id_of("orphan_a").await, id_of("orphan_b").await);
    let (role_num, dept_num) = (common::dec_id(&role), common::dec_id(&dept));

    // ── 塞孤儿行：指向不存在的角色 / 菜单 / 部门；再让 orphan_b 挂不存在的部门 ──
    let ghost = || common::new_row_id();
    let (ghost_role, ghost_menu, ghost_dept, ghost_role2, ghost_menu2, ghost_dept2) =
        (ghost(), ghost(), ghost(), ghost(), ghost(), ghost());
    sqlx::query("INSERT INTO admin_role (admin_id, role_id) VALUES (?, ?)")
        .bind(admin_a)
        .bind(ghost_role)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO role_menu (role_id, menu_id) VALUES (?, ?)")
        .bind(ghost_role2)
        .bind(ghost_menu)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO role_menu (role_id, menu_id) VALUES (?, ?)")
        .bind(role_num)
        .bind(ghost_menu2)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO role_dept (role_id, dept_id) VALUES (?, ?)")
        .bind(ghost_role)
        .bind(ghost_dept)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO role_dept (role_id, dept_id) VALUES (?, ?)")
        .bind(role_num)
        .bind(ghost_dept2)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE admin SET dept_id = ? WHERE id = ?")
        .bind(ghost())
        .bind(admin_b)
        .execute(&pool)
        .await
        .unwrap();

    // ── 手动触发日志保留任务（关系兜底搭它的车）─────────────────
    let (st, v) = call(&c, Method::GET, api("/jobs"), Some(&admin), None).await;
    assert_eq!(st, 200, "任务列表: {v}");
    let job = common::as_id(
        &v["data"]["list"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["code"] == "log_retention")
            .expect("有 log_retention 任务")["id"],
    );
    let mark = common::last_log_id(&pool, "log_retention").await;
    common::trigger(&c, &base, &job, &admin).await;
    let (status, msg, _ms) = common::wait_job_log(&pool, "log_retention", mark).await;
    assert_eq!(status, 1, "任务成功: {msg}");
    assert!(msg.contains("悬挂关联"), "执行摘要含清理结果: {msg}");

    // ── 孤儿清掉，有效关系与部门归属原样保留 ────────────────────
    let count = |sql: &'static str, a: i64, b: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(a)
                .bind(b)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        count("SELECT COUNT(*) FROM admin_role WHERE admin_id = ? AND role_id = ?", admin_a, ghost_role).await,
        0,
        "指向已删角色的授权行要清"
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM admin_role WHERE admin_id = ? AND role_id = ?", admin_a, role_num).await,
        1,
        "有效用户-角色关系不能动"
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM role_menu WHERE role_id = ? AND menu_id = ?", ghost_role2, ghost_menu).await,
        0,
        "指向已删角色的菜单关联要清"
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM role_menu WHERE role_id = ? AND menu_id = ?", role_num, common::dec_id(menu)).await,
        1,
        "有效角色-菜单关系不能动"
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM role_dept WHERE role_id = ? AND dept_id = ?", ghost_role, ghost_dept).await,
        0,
        "指向已删角色的部门关联要清"
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM role_dept WHERE role_id = ? AND dept_id = ?", role_num, dept_num).await,
        1,
        "有效角色-部门关系不能动"
    );
    let dept_of = |id: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT dept_id FROM admin WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(dept_of(admin_a).await, dept_num, "有效部门归属不能动");
    assert_eq!(dept_of(admin_b).await, 0, "指向已删部门的归属归零");
}
