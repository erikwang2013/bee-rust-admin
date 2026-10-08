// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 集成测试脚手架：随机端口起真实进程 + 真库（bee_admin_test）。
// 每个 `tests/*.rs` 都各自 `mod common;` 编译一份，用不到的辅助函数在那个二进制里
// 必然触发 dead_code —— 这是共享测试模块的常态，不是真死代码。
#![allow(dead_code)]
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use hashids::{Config, ConnectionConfig, Guard, HashidsManager};

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

// ── 对外 id：hashids 短字符串 ─────────────────────────────────
//
// bee_admin 是 bin crate，集成测试不能 use 它的代码，只能自己拿 hashids-rust 编解码。
// salt/最短长度与后端同源：直接读 conf/app.conf.test（键写在哪个节都认），读不到盐就报错，
// 不硬编码猜（猜错了表现是「解码全失败」，比报错难查）。

/// min_len 的兜底 8 与后端 `[app] hashids_min_len` 的代码默认值一致；
/// salt **不给兜底**：猜错盐只会得到「解码全失败」的哑谜，不如直接报缺键。
const MIN_LEN_FALLBACK: usize = 8;

fn conf_val(key: &str) -> Option<String> {
    let text = std::fs::read_to_string("conf/app.conf.test").ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn guard() -> &'static Guard {
    static G: OnceLock<Guard> = OnceLock::new();
    G.get_or_init(|| {
        let salt = conf_val("hashids_salt")
            .expect("conf/app.conf.test 缺 [app] hashids_salt（必须与后端同源，不能硬编码猜）");
        let min_len = conf_val("hashids_min_len")
            .and_then(|s| s.parse().ok())
            .unwrap_or(MIN_LEN_FALLBACK);
        let cfg = Config::new().default_connection("main").connection(
            "main",
            ConnectionConfig::new().salt(salt).min_hash_length(min_len),
        );
        Guard::from_manager(Arc::new(HashidsManager::new(cfg))).expect("hashids 初始化")
    })
}

/// 响应里的对外 id（hashid 串）→ 库里的数字 id（拼 SQL 绑定用）。
pub fn dec_id(s: &str) -> i64 {
    let ids = guard().decode(s);
    assert_eq!(ids.len(), 1, "hashid {s:?} 应解码出且仅一个数字（salt/长度是否与后端一致？）");
    ids[0] as i64
}

/// 数字 id → hashid 串：造「格式合法但库里不存在」的 id、把库里的自增 id 拼进 URL 用。
pub fn enc_id(n: u64) -> String {
    guard().encode(&[n])
}

/// 测试直接 INSERT 时自己造一个主键：库里的 id 由应用侧雪花生成（不是 AUTO_INCREMENT，
/// 不给人就得报 1364）。固定高位区间起步，跟雪花当前量级（~5e17）不撞。
pub fn new_row_id() -> i64 {
    use std::sync::atomic::{AtomicI64, Ordering};
    static NEXT: AtomicI64 = AtomicI64::new(1_000_000_000_000_000_000);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 从 JSON 取对外 id：直接透传字符串，**不解码再编码**（绕一圈没有意义还不稳）。
pub fn as_id(v: &serde_json::Value) -> String {
    v.as_str().unwrap_or_else(|| panic!("期望 hashid 字符串，实际 {v}")).to_string()
}

/// 当前登录者自己的 hashid（别假设「超管 id = 1」）。
pub async fn my_id(base: &str, token: &str) -> String {
    let v: serde_json::Value = reqwest::Client::new()
        .get(format!("{base}/api/v1/auth/profile"))
        .bearer_auth(token)
        .send()
        .await
        .expect("profile 请求失败")
        .json()
        .await
        .expect("profile 不是 JSON");
    as_id(&v["data"]["user"]["id"])
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

    #[test]
    fn hashid_roundtrip_is_deterministic() {
        // 编码是确定性的：同一个数字永远同一个串，且有最短长度
        let a = enc_id(999_999);
        assert_eq!(a, enc_id(999_999), "编码必须确定性");
        assert!(a.len() >= MIN_LEN_FALLBACK, "最短长度: {a}");
        assert_eq!(dec_id(&a), 999_999, "decode(encode(n)) == n");
        assert_ne!(enc_id(1), enc_id(2), "不同数字不同串");
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
    start_server_conf(|c| c).await
}

/// 同 [`start_server`]，但起服务前可改配置文本（如追加 `[security] scan = high`）。
/// 配置在启动时读取（拒绝非法值也是），所以需要不同配置的用例必须换一个进程。
pub async fn start_server_conf(patch: impl FnOnce(String) -> String) -> (Server, String) {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let conf = std::fs::read_to_string("conf/app.conf.test").unwrap();
    let conf = conf.replace("127.0.0.1:0", &format!("127.0.0.1:{port}"));
    let conf = patch(conf);
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
