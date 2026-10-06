// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 集成测试脚手架：随机端口起真实进程 + 真库（bee_admin_test）。
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub struct Server(pub Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// 取出 DSN 里的库名（`mysql://u:p@host:port/dbname?params` → `dbname`）。
pub fn db_name(dsn: &str) -> &str {
    dsn.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').nth(1))
        .map(|s| s.split('?').next().unwrap_or(""))
        .unwrap_or("")
}

pub fn dsn() -> Option<String> {
    match std::env::var("BEE_ADMIN_DB_DSN") {
        Ok(v) if !v.is_empty() => {
            // 护栏：集成测试会 DROP 全部表重建，绝不能指向生产库。
            // 所有测试都从这里取 DSN，被 spawn 的服务进程也继承同一个环境变量。
            let db = db_name(&v);
            assert!(
                db.ends_with("_test"),
                "拒绝执行：BEE_ADMIN_DB_DSN 指向的库 `{db}` 不是 *_test 库（这些测试会 DROP 全部表）"
            );
            Some(v)
        }
        _ => {
            eprintln!("跳过：未设置 BEE_ADMIN_DB_DSN");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_name_parses_dsn() {
        assert_eq!(db_name("mysql://root:pw@127.0.0.1:3306/bee_admin_test"), "bee_admin_test");
        assert_eq!(db_name("mysql://root:pw@127.0.0.1:3306/bee_admin_test?ssl-mode=DISABLED"), "bee_admin_test");
        assert_eq!(db_name("mysql://root:pw@127.0.0.1:3306"), "");
    }
}

/// 清库（服务启动时会 syncdb + seed 重建）。
pub async fn reset_db(dsn: &str) {
    let pool = sqlx::MySqlPool::connect(dsn).await.unwrap();
    for t in [
        "admin_role", "role_menu", "role_dept", "login_log", "audit_log", "menu", "role", "dept",
        "dict_item", "dict_type", "job_log", "job", "notice_read", "notice", "admin",
    ] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {t}")).execute(&pool).await.unwrap();
    }
}

/// 用随机端口起真实进程；返回 (进程守卫, base_url)。
pub async fn start_server() -> (Server, String) {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
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
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if reqwest::get(format!("{base}/api/v1/health")).await.is_ok() {
            return (server, base);
        }
        assert!(tokio::time::Instant::now() < deadline, "服务 15 秒内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// 登录拿 token。
pub async fn login(base: &str, user: &str, pass: &str) -> Option<String> {
    let r = reqwest::Client::new()
        .post(format!("{base}/api/v1/auth/login"))
        .json(&serde_json::json!({"username": user, "password": pass}))
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v: serde_json::Value = r.json().await.ok()?;
    v["data"]["token"].as_str().map(|s| s.to_string())
}
