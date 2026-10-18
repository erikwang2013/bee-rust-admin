// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! v16 集成测试的本地构造（F3 拆分：主文件只留用例）。

use crate::common;
use reqwest::Method;
use serde_json::Value;
use std::time::Duration;

/// 统一请求：返回 (状态码, JSON body)；断言消息带上 body，失败好定位。
pub(super) async fn call(
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
pub(super) fn menu<'a>(menus: &'a Value, perm: &str) -> Option<&'a Value> {
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
pub(super) async fn start_server_conf(extra: &str) -> (common::Server, String) {
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

pub(super) async fn job_log_count(pool: &sqlx::MySqlPool, code: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM job_log WHERE job_code = ?")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 第「重开进程」段原样搬出（F3 拆行数：断言零改动）。
pub(super) async fn restart_reapplies_seed_idempotently(
    server: common::Server,
    c: &reqwest::Client,
    pool: &sqlx::MySqlPool,
) {
    // 重开进程：菜单与任务补齐都是幂等的
    drop(server);
    let (_server2, base2) = common::start_server().await;
    let admin2 = common::login(&base2, "admin", "admin123").await.expect("重启后登录");
    let (st, v) = call(&c, Method::GET, format!("{base2}/api/v1/menus/tree"), Some(&admin2), None).await;
    assert_eq!(st, 200, "{v}");
    assert!(menu(&v["data"], "system:job:list").is_some(), "重启后菜单还在: {v}");
    let (st, v) = call(&c, Method::GET, format!("{base2}/api/v1/jobs"), Some(&admin2), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 8, "补的是缺的：7 个内置 + 库里原有的 ghost_job，不被删也不重复补: {v}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job WHERE code = 'log_retention'")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "内置任务只有一条: {n}");
    drop(_server2);
}

/// 第「[job] enabled=false」段原样搬出（F3 拆行数：断言零改动）。
pub(super) async fn disabled_scheduler_still_allows_manual_run(
    dsn: &str,
    c: &reqwest::Client,
    pool: &sqlx::MySqlPool,
) {
    // ── [job] enabled=false：不启动循环，但手动触发仍可用 ─────────
    common::reset_db(dsn).await;
    let (_server3, base3) = start_server_conf("[job]\nenabled = false").await;
    let admin3 = common::login(&base3, "admin", "admin123").await.expect("关定时后登录");
    let api3 = |p: &str| format!("{base3}/api/v1{p}");
    let (st, v) = call(&c, Method::GET, api3("/jobs"), Some(&admin3), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["total"], 7, "补齐不受开关影响（运维要能看到任务）: {v}");
    let rid = common::as_id(
        &v["data"]["list"].as_array().unwrap().iter()
            .find(|j| j["code"] == "log_retention").unwrap()["id"],
    );
    // 新库 last_run_at 全空 = 一旦循环启动，第一个 tick 就会跑；等 5 秒断言没跑
    tokio::time::sleep(Duration::from_secs(5)).await;
    let n = job_log_count(pool, "log_retention").await;
    assert_eq!(n, 0, "[job] enabled=false 时循环不该启动，实际跑了 {n} 次");
    let last: Option<chrono::NaiveDateTime> = sqlx::query_scalar("SELECT last_run_at FROM job WHERE code = 'log_retention'")
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(last.is_none(), "循环没启动，last_run_at 必须还是空: {last:?}");

    // 手动触发不看总开关（运维要能临时关掉定时、手动补跑）
    let mark = common::last_log_id(pool, "log_retention").await;
    common::trigger(&c, &base3, &rid, &admin3).await;
    let (status, msg, _ms) = common::wait_job_log(pool, "log_retention", mark).await;
    assert_eq!(status, 1, "关定时后手动触发仍要可用: {msg}");
    let n = job_log_count(pool, "log_retention").await;
    assert_eq!(n, 1, "手动跑一次写一条: {n}");
    // 再等一个 tick 的时间：手动跑过也不会让循环松口
    tokio::time::sleep(Duration::from_secs(3)).await;
    let n = job_log_count(pool, "log_retention").await;
    assert_eq!(n, 1, "循环仍然没启动: {n}");
}
