// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 起真实进程 + 真库（bee_admin_test）跑登录链路。需要 BEE_ADMIN_DB_DSN。
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn dsn() -> Option<String> {
    match std::env::var("BEE_ADMIN_DB_DSN") {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("跳过：未设置 BEE_ADMIN_DB_DSN");
            None
        }
    }
}

async fn start_server() -> (Server, String) {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    // 测试配置：把 app.conf.test 的 http_addr 改写到随机端口
    let conf = std::fs::read_to_string("conf/app.conf.test").unwrap();
    let conf = conf.replace("127.0.0.1:0", &format!("127.0.0.1:{port}"));
    let tmp = std::env::temp_dir().join(format!("bee_admin_test_{port}.conf"));
    std::fs::write(&tmp, conf).unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_bee_admin"))
        .env("BEE_ADMIN_CONF", &tmp)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("启动 bee_admin 失败");
    let server = Server(child);

    let base = format!("http://127.0.0.1:{port}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if reqwest::get(format!("{base}/api/v1/health")).await.is_ok() {
            return (server, base);
        }
        assert!(tokio::time::Instant::now() < deadline, "服务 20 秒内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn login_profile_menus_logout() {
    let Some(dsn) = dsn() else { return };

    // 干净库：删表后由服务启动时的 syncdb/seed 重建
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    for t in ["admin_role", "role_menu", "role_dept", "login_log", "menu", "role", "dept", "admin"] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {t}")).execute(&pool).await.unwrap();
    }
    drop(pool);

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

    // menus：目录 + 5 个菜单
    let r = client
        .get(format!("{base}/api/v1/auth/menus"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    let root = &body["data"][0];
    assert_eq!(root["path"], "/system");
    assert_eq!(root["children"].as_array().unwrap().len(), 5);

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

    // 登录记录：1 次失败 + 1 次成功
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM login_log")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
}
