// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 异步导出的「取消 / 进度 / 心跳 / 并发」契约（批次 E · E2-1）。
//!
//! 单开一个文件（没塞进 api_export_test.rs）：那边写满 ~460 行，家规 500 行上限；
//! 集成测试又是「一文件一 `#[tokio::test]`」（cargo 串行跑二进制，CI 可整包跑）。
//!
//! **时间敏感的地方一律不赌时序**：
//! - 「取消在批间生效」用 `LOCK TABLES login_log WRITE` 把执行者冻在**取数那一步**
//!   （表级写锁挡别的会话的读），于是「已认领、第一批还没写」是确定的现场，不是 sleep 猜的；
//! - 「并发上限 2」用同一把锁：3 个任务只可能 2 个被认领（第 3 个在等额度，连认领都没发生）；
//! - 「进度递增」不做「运行中抓到中间值」的断言（那才是赌博），钉的是**跨批不变量**：
//!   6000 行（> `csv::BATCH` 5000，必然两批）跑完 `progress_rows == row_count`、
//!   心跳不早于认领时刻，且跨批产物与同步导出**逐字节一致**。
mod common;

use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;

const EXP: &str = "/exports";
/// 测试账号口令（≥8 位、字母+数字，满足复杂度下限）。
const PASS: &str = "e2pass123";
/// 导出目录（`upload_dir` 是 cwd 相对的，conf/app.conf.test 里就是 target/test-uploads）。
const OUT: &str = "target/test-uploads/exports";

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
    (status, r.json().await.unwrap_or(Value::Null))
}

async fn download(c: &reqwest::Client, url: String, token: &str) -> (u16, Vec<u8>) {
    let r = c.get(url).bearer_auth(token).send().await.expect("下载请求失败");
    (r.status().as_u16(), r.bytes().await.unwrap().to_vec())
}

async fn sync_csv(c: &reqwest::Client, url: String, token: &str) -> Vec<u8> {
    let r = c.get(url).bearer_auth(token).send().await.expect("同步导出请求失败");
    assert_eq!(r.status(), 200, "同步导出必须仍可用（回归）");
    r.bytes().await.unwrap().to_vec()
}

/// 任务的四元现场：状态 / 行数 / 进度 / 提示。
async fn snapshot(pool: &sqlx::MySqlPool, id: i64) -> (i8, i64, i64, String) {
    sqlx::query_as("SELECT status, row_count, progress_rows, msg FROM export_task WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("任务行还在")
}

/// 轮询到状态落进 `want`，超时把实际现场报出来（所有「等一等」都有上限）。
async fn wait_status(pool: &sqlx::MySqlPool, id: i64, want: &[i8]) -> (i8, i64, i64, String) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let now = snapshot(pool, id).await;
        if want.contains(&now.0) {
            return now;
        }
        assert!(tokio::time::Instant::now() < deadline, "等状态 {want:?} 超时，现在 {now:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn exists(p: &str) -> bool {
    std::path::Path::new(p).exists()
}

/// 一个任务在盘上**所有**的半份（E2 修复轮起 `.part` 名字带代次：`{id}.{代次}.csv.part`，
/// 旧的 `{id}.csv.part` 也认）。按「{id}. 前缀 + .csv.part 后缀」扫，不赌具体代次。
fn parts_of(id: i64) -> Vec<std::path::PathBuf> {
    let prefix = format!("{id}.");
    let Ok(rd) = std::fs::read_dir(OUT) else { return Vec::new() };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with(&prefix) && name.ends_with(".csv.part")
        })
        .collect()
}

/// 等一条**取消行**的收尾落定：半份与成品都不在盘上、`file_path` 也擦干净了。
/// 执行者收尾是「改名 → 抢终态 → 抢不到就撤成品」，松开表锁之后这几步还在跑 ——
/// 所以要**等到**（等到是确定的；等不到才是真漏了）。
async fn wait_cancel_settled(pool: &sqlx::MySqlPool, id: i64) {
    let csv = format!("{OUT}/{id}.csv");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let fp: String = sqlx::query_scalar("SELECT file_path FROM export_task WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        let parts = parts_of(id);
        if fp.is_empty() && !exists(&csv) && parts.is_empty() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "取消的收尾 30 秒内没落定：file_path={fp:?} csv={} parts={parts:?}",
            exists(&csv)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 等任务的半份文件出现（上限 10 秒）。认领（status=1、心跳已写）与「.part 已落盘」之间
/// 隔着查发起人、建目录、open 几步 —— 直接断言两者同时成立就是赌时序。这里的现场是
/// 「执行者被表锁冻在取数那一步」，文件只可能**出现**（冻住之后不会再消失），
/// 所以等一下是确定的，不是碰运气。
async fn wait_part(id: i64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while parts_of(id).is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "10 秒内没等到任务 {id} 的 .part");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 认领中的行数（`LOCK TABLES` 期间它们卡在取数那一步）。
async fn count_running(pool: &sqlx::MySqlPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM export_task WHERE status = 1")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 按 code 找注册任务。
async fn job_id(c: &reqwest::Client, base: &str, token: &str, code: &str) -> String {
    let (st, v) = call(c, Method::GET, format!("{base}/api/v1/jobs?size=100"), Some(token), None).await;
    assert_eq!(st, 200, "任务列表: {v}");
    v["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["code"] == code)
        .unwrap_or_else(|| panic!("任务列表里应有 {code}: {v}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// 对外 id（hashid 字符串）。**别把 JSON 值直接塞进 `format!`** —— `Value` 的 Display
/// 给字符串加引号，拼出来是 `/exports/"abc"/cancel`，后端只回「无效的 id」（踩过一次）。
fn sid(t: &Value) -> String {
    t["data"]["id"].as_str().expect("响应里应有 data.id").to_string()
}

fn row<'a>(list: &'a Value, id: &str) -> &'a Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("列表里应有任务 {id}: {list}"))
}

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

/// 直接写库造 n 行登录记录（不走登录接口：n 次 Argon2 校验慢到不可接受）。
/// 6000 行是为了跨过 `csv::BATCH = 5000` 的批界 —— 异步导出必然分两批取数。
async fn bulk_login_logs(pool: &sqlx::MySqlPool, username: &str, n: i64, at: chrono::NaiveDateTime) {
    let mut id = common::new_row_id();
    let at = at.format("%Y-%m-%d %H:%M:%S").to_string();
    for _ in 0..(n / 1000) {
        let mut sql = String::from(
            "INSERT INTO login_log (id, admin_id, username, ip, user_agent, status, msg, created_at) VALUES ",
        );
        for i in 0..1000 {
            if i > 0 {
                sql.push(',');
            }
            sql.push_str(&format!("({id}, 0, '{username}', '127.0.0.1', 'bulk', 0, '批量造数', '{at}')"));
            id += 1;
        }
        sqlx::query(&sql).execute(pool).await.expect("批量造登录记录");
    }
}

#[tokio::test]
async fn export_cancel_progress_and_heartbeat() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    // 关掉调度循环：本用例精确控制「什么时候跑」（调度 tick 会半路抢任务）
    let (_srv, base) = common::start_server_conf(|c| format!("{c}\n[job]\nenabled = false\n")).await;
    let c = reqwest::Client::new();
    let url = |p: &str| format!("{base}/api/v1{p}");
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");
    let run_id = job_id(&c, &base, &admin, "export_run").await;
    let clean_id = job_id(&c, &base, &admin, "export_clean").await;
    // 时戳在 Rust 侧给（sqlx 会话时区是 UTC，MySQL NOW() 会偏 8 小时）
    let past = chrono::NaiveDateTime::parse_from_str("2020-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    std::fs::create_dir_all(OUT).expect("导出目录");

    // ── 1. 取消待处理的任务：端点语义（200 / 409 / 403 / 404 + 下载 400）──
    let (st, t1) = call(&c, Method::POST, url(EXP), Some(&admin), Some(json!({"kind": "login-log"}))).await;
    assert_eq!(st, 200, "{t1}");
    let id1 = common::dec_id(&common::as_id(&t1["data"]["id"]));
    let (st, v) = call(&c, Method::POST, url(&format!("{EXP}/{}/cancel", sid(&t1))), Some(&admin), None).await;
    assert_eq!(st, 200, "取消待处理的任务: {v}");
    assert_eq!(v["data"]["canceled"], 1, "{v}");
    let (status, _, prog, msg) = snapshot(&pool, id1).await;
    assert_eq!((status, prog), (4, 0), "取消 = 置 4，进度停在 0（还没跑过）: {msg}");
    assert_eq!(msg, "已取消", "{msg}");
    let fin: Option<chrono::NaiveDateTime> =
        sqlx::query_scalar("SELECT finished_at FROM export_task WHERE id = ?").bind(id1).fetch_one(&pool).await.unwrap();
    assert!(fin.is_some(), "取消也要盖完成时刻，列表上那两列才不空着");

    // 重复取消 → 409（请求没毛病，是行已经结束了）
    let (st, v) = call(&c, Method::POST, url(&format!("{EXP}/{}/cancel", sid(&t1))), Some(&admin), None).await;
    assert_eq!(st, 409, "已经结束的不能取消: {v}");
    assert_eq!(v["err"], "export.not_cancelable", "前端要能翻成中英文案: {v}");
    // 不存在的任务 → 404（id 是 hashid，猜不出来，这里只是造一个格式合法的）
    let (st, _) = call(&c, Method::POST, url(&format!("{EXP}/{}/cancel", common::enc_id(999_999_997))), Some(&admin), None).await;
    assert_eq!(st, 404, "不存在的任务");
    // 取消掉的没有文件：下载给 400 而不是一份空 CSV
    let (st, body) = download(&c, url(&format!("{EXP}/{}/download", sid(&t1))), &admin).await;
    assert_eq!(st, 400, "取消的任务不能下载");
    assert!(String::from_utf8_lossy(&body).contains("已取消"), "提示要说清是取消: {}", String::from_utf8_lossy(&body));
    // 列表：状态筛 4 能查出来（前端那个筛选框要真的有东西可筛）
    let (st, list) = call(&c, Method::GET, url(&format!("{EXP}?status=4&size=50")), Some(&admin), None).await;
    assert_eq!(st, 200, "{list}");
    let lrow = row(&list["data"]["list"], t1["data"]["id"].as_str().unwrap());
    assert_eq!(lrow["status"], 4, "筛出来的是取消行: {lrow}");
    assert_eq!(lrow["progress_rows"], 0, "字段在列表里（前端进度列靠它）: {lrow}");

    // 只挂导出中心码的另一个账号：取消**别人**的任务 → 403（能看到码也不许动别人的）
    let (st, menus) = call(&c, Method::GET, url("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200, "菜单树: {menus}");
    let center = menu_id(&menus["data"], "system:export:list").expect("内置菜单含导出中心");
    let (st, role) = call(&c, Method::POST, url("/roles"), Some(&admin),
        Some(json!({"name": "e2导出员", "code": "e2exporter", "data_scope": 1, "status": 1}))).await;
    assert_eq!(st, 200, "{role}");
    let rid = common::as_id(&role["data"]["id"]);
    let (st, v) = call(&c, Method::PUT, url(&format!("/roles/{rid}/menus")), Some(&admin),
        Some(json!({"menu_ids": [center]}))).await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = call(&c, Method::POST, url("/admins"), Some(&admin),
        Some(json!({"username": "e2_other", "password": PASS, "role_ids": [rid]}))).await;
    assert_eq!(st, 200, "{v}");
    let other = common::login(&base, "e2_other", PASS).await.expect("导出员登录");
    let (st, v) = call(&c, Method::POST, url(&format!("{EXP}/{}/cancel", sid(&t1))), Some(&other), None).await;
    assert_eq!(st, 403, "别人的任务不能取消: {v}");

    // ── 2. 取消在**批间**生效（把执行者冻在取数那一步，不靠 sleep 猜）──
    // `LOCK TABLES login_log WRITE` 会顶住执行者的取数 SELECT（表级写锁挡别的会话的读），
    // 而认领与心跳/进度回写都写在 export_task 上 —— 锁只冻住「取数」，冻不住认领。
    // 于是「已认领、第一批还没写」是个**确定**的现场。
    // `raw_sql` 而不是 `query`：LOCK/UNLOCK TABLES 走不了预处理协议（1295），必须用文本协议。
    let mut lock = pool.acquire().await.unwrap();
    sqlx::raw_sql("LOCK TABLES login_log WRITE").execute(&mut *lock).await.expect("锁表（要 root + 会话上没有活跃事务）");
    let (st, t2) = call(&c, Method::POST, url(EXP), Some(&admin), Some(json!({"kind": "login-log"}))).await;
    assert_eq!(st, 200, "{t2}");
    let id2 = common::dec_id(&common::as_id(&t2["data"]["id"]));
    let before = common::last_log_id(&pool, "export_run").await;
    common::trigger(&c, &base, &run_id, &admin).await;
    // 派发式：job 自己**已经收工**（不占住循环），任务还在跑 —— 这正是 E2-1 要的形态
    let (jstatus, jmsg, _) = common::wait_job_log(&pool, "export_run", before).await;
    assert_eq!(jstatus, 1, "{jmsg}");
    assert_eq!(jmsg, "导出任务 1 个（已派发，并发上限 2）", "派发完就返回: {jmsg}");

    let (status, _, _, _) = wait_status(&pool, id2, &[1]).await;
    assert_eq!(status, 1, "被锁顶住的任务停在「执行中」");
    let (hb, started): (Option<chrono::NaiveDateTime>, Option<chrono::NaiveDateTime>) =
        sqlx::query_as("SELECT heartbeat_at, started_at FROM export_task WHERE id = ?")
            .bind(id2).fetch_one(&pool).await.unwrap();
    assert!(hb.is_some() && hb >= started, "运行中就该有心跳（判活只看它，M1）: {hb:?} / {started:?}");
    let (_, _, prog2, _) = snapshot(&pool, id2).await;
    assert_eq!(prog2, 0, "第一批还没落盘：进度回写发生在批界");
    wait_part(id2).await; // 跑起来的任务先落 .part（冻结现场的一部分）

    let (st, v) = call(&c, Method::POST, url(&format!("{EXP}/{}/cancel", sid(&t2))), Some(&admin), None).await;
    assert_eq!((st, v["data"]["canceled"].as_i64()), (200, Some(1)), "执行中的也能取消: {v}");
    sqlx::raw_sql("UNLOCK TABLES").execute(&mut *lock).await.unwrap();
    drop(lock);
    // 解锁后执行者接着跑：写完第一批 → 下一轮取数**之前**先探取消（谓词 status = 1 命中 0 行）
    // → 收手。.part 是看守在 drop 里删的 —— 它消失就是「批间生效」的可见信号
    wait_cancel_settled(&pool, id2).await;
    assert!(parts_of(id2).is_empty(), "取消后不留半份: {:?}", parts_of(id2));
    assert!(!exists(&format!("{OUT}/{id2}.csv")), "取消的任务不许有成品");
    let (status, rows2, prog2b, msg2) = snapshot(&pool, id2).await;
    assert_eq!((status, rows2, prog2b), (4, 0, 0), "执行者收手后行仍停在 4，终态没被改写: {msg2}");
    assert_eq!(msg2, "已取消", "{msg2}");
    let fp: String = sqlx::query_scalar("SELECT file_path FROM export_task WHERE id = ?")
        .bind(id2).fetch_one(&pool).await.unwrap();
    assert!(fp.is_empty(), "取消的行不该指着一个不存在的文件: {fp:?}");

    // ── 3. 心跳保活（M1）：跑得久但心跳还热的任务不算死，它的 .part 也不许被收 ──
    let (st, t3) = call(&c, Method::POST, url(EXP), Some(&admin), Some(json!({"kind": "login-log"}))).await;
    assert_eq!(st, 200, "{t3}");
    let id3 = common::dec_id(&common::as_id(&t3["data"]["id"]));
    let fresh = chrono::Local::now().naive_local();
    let old = fresh - chrono::Duration::seconds(3600);
    let fake3 = format!("{OUT}/{id3}.csv.part");
    std::fs::write(&fake3, b"x").unwrap();
    // 现场：认领于 1 小时前、心跳在刚刚（= 跑了一小时还活着的任务），且**已经过期** ——
    // 过期是故意的：让「收不收 .part」只由心跳决定，别被 expire_at 提前挡住
    sqlx::query("UPDATE export_task SET status = 1, started_at = ?, heartbeat_at = ?, expire_at = ? WHERE id = ?")
        .bind(old).bind(fresh).bind(past).bind(id3)
        .execute(&pool).await.unwrap();
    let before = common::last_log_id(&pool, "export_run").await;
    common::trigger(&c, &base, &run_id, &admin).await;
    let (jstatus, jmsg, _) = common::wait_job_log(&pool, "export_run", before).await;
    assert_eq!(jstatus, 1, "{jmsg}");
    assert_eq!(jmsg, "导出任务 0 个", "心跳还热的执行中任务不算「进程崩过」: {jmsg}");
    let (status3, _, _, msg3) = snapshot(&pool, id3).await;
    assert_eq!(status3, 1, "没重跑，行原样: {msg3}");

    let before = common::last_log_id(&pool, "export_clean").await;
    common::trigger(&c, &base, &clean_id, &admin).await;
    let (jstatus, jmsg, _) = common::wait_job_log(&pool, "export_clean", before).await;
    assert_eq!(jstatus, 1, "{jmsg}");
    assert!(exists(&fake3), "清理也不许收活任务的半份（M1）: {fake3}");

    // 心跳断了（> 600s）才轮到重跑：恢复机制没被这次改动顺手改掉
    sqlx::query("UPDATE export_task SET heartbeat_at = ? WHERE id = ?")
        .bind(old).bind(id3)
        .execute(&pool).await.unwrap();
    common::trigger(&c, &base, &run_id, &admin).await;
    let (status3b, rows3, prog3, msg3b) = wait_status(&pool, id3, &[2, 3]).await;
    assert_eq!(status3b, 2, "心跳断了要能重跑: {msg3b}");
    assert_eq!(rows3, prog3, "跑完时进度补齐到行数（终态那一写带上进度）");
    assert!(exists(&format!("{OUT}/{id3}.csv")), "重跑产出了完整成品");
    // 旧代次的 .part 不会被新执行者挪用/截断（代次隔离），它成了终态行的孤儿，
    // 留给清理收 —— 这正是 I2 修复后「旧执行者留下的垃圾」该走的路。
    assert_eq!(parts_of(id3), vec![std::path::PathBuf::from(&fake3)],
        "重跑用的是自己的代次，不碰旧半份: {:?}", parts_of(id3));
    let before = common::last_log_id(&pool, "export_clean").await;
    common::trigger(&c, &base, &clean_id, &admin).await;
    let (jstatus, jmsg, _) = common::wait_job_log(&pool, "export_clean", before).await;
    assert_eq!(jstatus, 1, "{jmsg}");
    assert!(!exists(&fake3), "旧代次的孤儿 .part 最终由清理收走: {fake3}");

    // ── 4. 跨批运行：进度收尾 + 跨批产物与同步导出一致 ──
    // 6000 行 > BATCH(5000)：异步导出必然分两批取数（keyset 翻页），产物必须与一次性
    // 流式的同步导出**逐字节一致** —— 这是「分页取数」的正确性证明，也是进度列的现场。
    bulk_login_logs(&pool, "e2_bulk", 6000, fresh - chrono::Duration::days(1)).await;
    let (st, t4) = call(&c, Method::POST, url(EXP), Some(&admin),
        Some(json!({"kind": "login-log", "params": {"username": "e2_bulk"}}))).await;
    assert_eq!(st, 200, "{t4}");
    let id4 = common::dec_id(&common::as_id(&t4["data"]["id"]));
    common::trigger(&c, &base, &run_id, &admin).await;
    let (status4, rows4, prog4, msg4) = wait_status(&pool, id4, &[2, 3]).await;
    assert_eq!((status4, rows4), (2, 6000), "两批取完，行数不多不少: {msg4}");
    assert_eq!(prog4, 6000, "跑完的进度 = 行数（列不许留 0 或半截值）: {msg4}");
    let (hb4, started4): (Option<chrono::NaiveDateTime>, Option<chrono::NaiveDateTime>) =
        sqlx::query_as("SELECT heartbeat_at, started_at FROM export_task WHERE id = ?")
            .bind(id4).fetch_one(&pool).await.unwrap();
    assert!(hb4.is_some() && hb4 >= started4, "批界的 beat 刷新过心跳: {hb4:?} / {started4:?}");
    let (st, bytes) = download(&c, url(&format!("{EXP}/{}/download", sid(&t4))), &admin).await;
    assert_eq!(st, 200, "6000 行的成品能下");
    assert_eq!(bytes, sync_csv(&c, url("/login-logs/export?username=e2_bulk"), &admin).await,
        "跨批分页取数 == 一次性流式（6000 行两批）");

    // ── 5. 取消的行怎么被清理（衔接 1）──
    // (a) 没过期但被取消：残留 .part 要被 `export_clean` 收走，记录留着（还没到期）
    let (st, t5) = call(&c, Method::POST, url(EXP), Some(&admin), Some(json!({"kind": "login-log"}))).await;
    assert_eq!(st, 200, "{t5}");
    let id5 = common::dec_id(&common::as_id(&t5["data"]["id"]));
    let (st, v) = call(&c, Method::POST, url(&format!("{EXP}/{}/cancel", sid(&t5))), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    let fake5 = format!("{OUT}/{id5}.csv.part");
    std::fs::write(&fake5, b"x").unwrap();
    let before = common::last_log_id(&pool, "export_clean").await;
    common::trigger(&c, &base, &clean_id, &admin).await;
    let (jstatus, jmsg, _) = common::wait_job_log(&pool, "export_clean", before).await;
    assert_eq!(jstatus, 1, "{jmsg}");
    assert!(!exists(&fake5), "取消行的孤儿 .part 要被收走: {fake5}");
    let n5: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_task WHERE id = ?")
        .bind(id5).fetch_one(&pool).await.unwrap();
    assert_eq!(n5, 1, "没过期的取消记录留着（用户还看得见自己取消过什么）");

    // (b) 取消 + 过期：连记录带文件一起清（`clean` 的名单里必须有 4）
    let (st, t6) = call(&c, Method::POST, url(EXP), Some(&admin), Some(json!({"kind": "login-log"}))).await;
    assert_eq!(st, 200, "{t6}");
    let id6 = common::dec_id(&common::as_id(&t6["data"]["id"]));
    let (st, v) = call(&c, Method::POST, url(&format!("{EXP}/{}/cancel", sid(&t6))), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    // 现场：取消那一行在盘上留了个按约定的 {id}.csv（file_path 空着 → 兜底路径）
    let fake6 = format!("{OUT}/{id6}.csv");
    std::fs::write(&fake6, b"x").unwrap();
    sqlx::query("UPDATE export_task SET expire_at = ? WHERE id = ?")
        .bind(past).bind(id6).execute(&pool).await.unwrap();
    let before = common::last_log_id(&pool, "export_clean").await;
    common::trigger(&c, &base, &clean_id, &admin).await;
    let (jstatus, jmsg, _) = common::wait_job_log(&pool, "export_clean", before).await;
    assert_eq!(jstatus, 1, "{jmsg}");
    assert!(!exists(&fake6), "取消行的孤儿成品也要清: {fake6}");
    let n6: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_task WHERE id = ?")
        .bind(id6).fetch_one(&pool).await.unwrap();
    assert_eq!(n6, 0, "过期的取消记录连记录一起清（老 clean 的名单里没有 4，会永远删不掉）");

    // ── 6. 并发上限 2（确定性）：锁着库，3 个任务里只可能有 2 个被认领 ──
    // 没有上限的话三个都会认领（认领只写 export_task，表锁挡不住）；有上限时第 3 个还在
    // 等额度 —— 连认领都没发生，所以它必须还是「待处理」，盘上也不该有它的 .part。
    let mut lock = pool.acquire().await.unwrap();
    sqlx::raw_sql("LOCK TABLES login_log WRITE").execute(&mut *lock).await.unwrap();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let (st, t) = call(&c, Method::POST, url(EXP), Some(&admin), Some(json!({"kind": "login-log"}))).await;
        assert_eq!(st, 200, "{t}");
        ids.push(common::dec_id(&common::as_id(&t["data"]["id"])));
    }
    let before = common::last_log_id(&pool, "export_run").await;
    common::trigger(&c, &base, &run_id, &admin).await;
    let (jstatus, jmsg, _) = common::wait_job_log(&pool, "export_run", before).await;
    assert_eq!(jstatus, 1, "{jmsg}");
    assert_eq!(jmsg, "导出任务 3 个（已派发，并发上限 2）", "{jmsg}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while count_running(&pool).await < 2 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let running: Vec<i64> = sqlx::query_scalar("SELECT id FROM export_task WHERE status = 1")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(running.len(), 2, "并发上限 2：第 3 个必须还在等额度（认领都没发生）");
    let waiting: Vec<i64> = sqlx::query_scalar("SELECT id FROM export_task WHERE status = 0")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(waiting.len(), 1, "排队那一个还是待处理: {waiting:?}");
    assert!(parts_of(waiting[0]).is_empty(), "没认领就不该碰盘: {:?}", parts_of(waiting[0]));

    // 收摊：取消那两个在跑的，解锁 —— 它们各自在批间收手（半份不留）
    for id in &running {
        let (st, v) = call(&c, Method::POST, url(&format!("{EXP}/{}/cancel", common::enc_id(*id as u64))), Some(&admin), None).await;
        assert_eq!(st, 200, "取消在跑的 {id}: {v}");
    }
    sqlx::raw_sql("UNLOCK TABLES").execute(&mut *lock).await.unwrap();
    drop(lock);
    for id in &running {
        wait_cancel_settled(&pool, *id).await;
        assert!(parts_of(*id).is_empty(), "取消的在跑任务不留半份: {:?}", parts_of(*id));
        let (status, _, _, _) = snapshot(&pool, *id).await;
        assert_eq!(status, 4, "收手后停在 4");
    }
    // 额度一放出来，排队那个才轮到它：认领 → 跑完（排队语义的另一半）
    let (status7, rows7, prog7, msg7) = wait_status(&pool, waiting[0], &[2, 3]).await;
    assert_eq!(status7, 2, "排队的任务最终也要跑完（不是被丢下）: {msg7}");
    assert!(rows7 > 0, "库里那些登录记录都该在: {msg7}");
    assert_eq!(prog7, rows7, "排队跑完的也把进度补齐: {msg7}");
}
