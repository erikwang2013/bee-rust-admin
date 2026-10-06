// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 起真实进程 + 真库（bee_admin_test）跑登录链路。需要 BEE_ADMIN_DB_DSN。
mod common;

use common::{dsn, reset_db, start_server};

#[tokio::test]
async fn login_profile_menus_logout() {
    // dsn() 内含生产库护栏：DSN 不指向 *_test 库时直接拒绝执行
    let Some(dsn) = dsn() else { return };

    // 干净库：删表后由服务启动时的 syncdb/seed 重建
    reset_db(&dsn).await;

    let (_server, base) = start_server().await;
    let client = reqwest::Client::new();

    // 登录失败：密码错
    let r = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&serde_json::json!({"username": "admin", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_ne!(body["code"], 0);

    // 登录成功
    let r = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&serde_json::json!({"username": "admin", "password": "admin123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["code"], 0);
    let token = body["data"]["token"].as_str().unwrap().to_string();
    assert_eq!(body["data"]["user"]["is_super"], true);

    // 未带 token → 401
    let r = client.get(format!("{base}/api/v1/auth/profile")).send().await.unwrap();
    assert_eq!(r.status(), 401);

    // profile
    let r = client
        .get(format!("{base}/api/v1/auth/profile"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["data"]["user"]["username"], "admin");
    assert_eq!(body["data"]["perms"][0], "*:*:*");

    // menus：目录 + 7 个菜单（v1.2 多了「操作日志」，v1.5 多了「字典管理」）
    let r = client
        .get(format!("{base}/api/v1/auth/menus"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    let root = &body["data"][0];
    assert_eq!(root["path"], "/system");
    assert_eq!(root["children"].as_array().unwrap().len(), 7);

    // 登出后旧 token 失效（token_version +1）
    let r = client
        .post(format!("{base}/api/v1/auth/logout"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = client
        .get(format!("{base}/api/v1/auth/profile"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // 登录记录：1 次失败 + 1 次成功 + 1 次退出
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM login_log")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 3, "失败 / 成功 / 退出各一条");
    let (msg,): (String,) = sqlx::query_as("SELECT msg FROM login_log ORDER BY id DESC LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(msg, "退出登录", "最后一条是退出登录");
}
