// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 异步导出（扩展工程项 4）：导出中心的后端契约。
//!
//! 一个用例钉住施工图验收 + 负控：
//! 1. 发起 → 任务跑完 → 文件可下，且**内容与同步导出逐字节一致**（抽公共层的意义）；
//! 2. 过期清理生效：记录与文件一起消失，且不误删未过期的；
//! 3. 权限不越权：没有日志的 `:list` 码发不了；别人的任务下载 / 删除都 403；
//! 4. 未完成的下载 400、未知 kind 400、参数类型不对 400、任务不存在 404。
//! 另加审计留痕（写路径必须有）。起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;

const EXP: &str = "/exports";
/// 测试账号口令（≥8 位、字母+数字，满足复杂度下限）。
const PASS: &str = "t4pass123";

/// 统一请求：返回 (状态码, JSON body)。
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

/// 原始字节下载：CSV 要逐字节比对，不能过 JSON（也不是 JSON）。
async fn download(c: &reqwest::Client, url: String, token: &str) -> (u16, Vec<u8>) {
    let r = c.get(url).bearer_auth(token).send().await.expect("下载请求失败");
    let status = r.status().as_u16();
    (status, r.bytes().await.unwrap().to_vec())
}

/// 同步导出端点取原始字节（与异步文件比对的基准）。
async fn sync_csv(c: &reqwest::Client, url: String, token: &str) -> Vec<u8> {
    let r = c.get(url).bearer_auth(token).send().await.expect("同步导出请求失败");
    assert_eq!(r.status(), 200, "同步导出必须仍可用（回归）");
    r.bytes().await.unwrap().to_vec()
}

/// 等导出任务到终态（2 完成 / 3 失败）。
async fn wait_task(pool: &sqlx::MySqlPool, id: i64) -> (i8, i64, String) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let row: (i8, i64, String) =
            sqlx::query_as("SELECT status, row_count, msg FROM export_task WHERE id = ?")
                .bind(id)
                .fetch_one(pool)
                .await
                .expect("任务行还在");
        if row.0 == 2 || row.0 == 3 || tokio::time::Instant::now() > deadline {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// 菜单树里按权限码取菜单 id（按码找，不按名字）。
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

/// 按 code 找注册任务（项 9 之后任务走 `JOBS` 代码注册 + 启动补齐，界面只能改间隔）。
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

/// 列表里按 id 找一行；找不到直接失败（细节：任务是要么在要么不在的东西）。
fn row<'a>(list: &'a Value, id: &str) -> &'a Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("列表里应有任务 {id}: {list}"))
}

#[tokio::test]
async fn export_tasks_run_download_and_expire() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    // 关掉调度循环：本用例精确控制「什么时候跑」（调度 tick 会半路抢任务，
    // 触发还会撞上「已在执行中」的 409）；手动触发不依赖调度，照样能用
    let (_srv, base) = common::start_server_conf(|c| format!("{c}\n[job]\nenabled = false\n")).await;
    let c = reqwest::Client::new();
    let url = |p: &str| format!("{base}/api/v1{p}");
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");

    // ── 造导出用的数据：2 个部门（审计行 module=dept）+ 2 次失败登录 ──
    for name in ["ex4甲部", "ex4乙部"] {
        let (st, v) = call(&c, Method::POST, url("/depts"), Some(&admin),
            Some(json!({"parent_id": 0, "name": name}))).await;
        assert_eq!(st, 200, "{v}");
    }
    for _ in 0..2 {
        assert!(common::login(&base, "ex4_ghost", "wrong-pass").await.is_none(), "口令错必须登录失败");
    }

    // 未知 kind：400（路由在不在，这条先说话）
    let (st, v) = call(&c, Method::POST, url(EXP), Some(&admin), Some(json!({"kind": "nope"}))).await;
    assert_eq!(st, 400, "未知 kind: {v}");

    // 筛选参数序列化后超过列宽（params 是 varchar(1024)）：发起时 400，不能等 INSERT 炸 500
    let huge = json!({"kind": "audit-log", "params": {"username": "x".repeat(1100)}});
    let (st, v) = call(&c, Method::POST, url(EXP), Some(&admin), Some(huge)).await;
    assert_eq!(st, 400, "超长 params 要当场拒: {v}");
    assert_eq!(v["err"], "common.too_long", "错误码要能翻成「最多 1024 字符」: {v}");

    // ── 只挂导出中心码的角色：发不了日志类导出（数据本身还得有查看码）──
    let (st, menus) = call(&c, Method::GET, url("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200, "菜单树: {menus}");
    let center = menu_id(&menus["data"], "system:export:list").expect("内置菜单含导出中心");
    let rm = menu_id(&menus["data"], "system:export:remove").expect("内置含删除按钮码");
    let (st, role) = call(&c, Method::POST, url("/roles"), Some(&admin),
        Some(json!({"name": "ex4导出员", "code": "ex4exporter", "data_scope": 1, "status": 1}))).await;
    assert_eq!(st, 200, "{role}");
    let rid = common::as_id(&role["data"]["id"]);
    let (st, v) = call(&c, Method::PUT, url(&format!("/roles/{rid}/menus")), Some(&admin),
        Some(json!({"menu_ids": [center, rm]}))).await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = call(&c, Method::POST, url("/admins"), Some(&admin),
        Some(json!({"username": "ex4_center", "password": PASS, "role_ids": [rid]}))).await;
    assert_eq!(st, 200, "{v}");
    let center_tok = common::login(&base, "ex4_center", PASS).await.expect("导出员登录");

    for kind in ["audit-log", "login-log"] {
        let (st, v) = call(&c, Method::POST, url(EXP), Some(&center_tok), Some(json!({"kind": kind}))).await;
        assert_eq!(st, 403, "只有导出中心码、没有 {kind} 的查看码：必须 403: {v}");
    }

    // ── 超管发起两个任务（带筛选参数）──
    let (st, t_login) = call(&c, Method::POST, url(EXP), Some(&admin),
        Some(json!({"kind": "login-log", "params": {"username": "ex4_ghost"}}))).await;
    assert_eq!(st, 200, "{t_login}");
    assert_eq!(t_login["data"]["status"], 0, "新建 = 待处理: {t_login}");
    let login_task = common::as_id(&t_login["data"]["id"]);

    // 未完成先下载：400（而不是给一份空文件）
    let (st, _) = download(&c, url(&format!("{EXP}/{login_task}/download")), &admin).await;
    assert_eq!(st, 400, "未完成的下载必须 400");

    let (st, t_audit) = call(&c, Method::POST, url(EXP), Some(&admin),
        Some(json!({"kind": "audit-log", "params": {"module": "dept"}}))).await;
    assert_eq!(st, 200, "{t_audit}");
    let audit_task = common::as_id(&t_audit["data"]["id"]);

    // 负控：参数类型不对 / 任务不存在
    let (st, v) = call(&c, Method::POST, url(EXP), Some(&admin),
        Some(json!({"kind": "login-log", "params": {"status": "不是整数"}}))).await;
    assert_eq!(st, 400, "参数类型不对要在发起时当场拒（别等任务跑挂）: {v}");
    let (st, _) = download(&c, url(&format!("{EXP}/{}/download", common::enc_id(999_999_999))), &admin).await;
    assert_eq!(st, 404, "不存在的任务");

    // ── 跑导出任务（既有 job 循环 + 手动触发同一条路）──
    let run_id = job_id(&c, &base, &admin, "export_run").await;
    let before = common::last_log_id(&pool, "export_run").await;
    common::trigger(&c, &base, &run_id, &admin).await;
    let (status, msg, _ms) = common::wait_job_log(&pool, "export_run", before).await;
    assert_eq!(status, 1, "导出任务执行必须成功: {msg}");
    let (status, rows, msg) = wait_task(&pool, common::dec_id(&login_task)).await;
    assert_eq!((status, rows), (2, 2), "后台跑完：任务到完成态且行数对: {msg}");

    // 列表：自己的任务（超管也只列自己的）
    let (st, list) = call(&c, Method::GET, url(&format!("{EXP}?size=50")), Some(&admin), None).await;
    assert_eq!(st, 200, "{list}");
    let lrow = row(&list["data"]["list"], &login_task);
    assert_eq!(lrow["status"], 2, "跑完 = 已完成: {lrow}");
    assert_eq!(lrow["kind"], "login-log", "{lrow}");
    assert_eq!(lrow["row_count"], 2, "两次失败登录都在: {lrow}");

    // **内容与同步导出逐字节一致**（抽公共层就是要这个）
    let (st, async_csv) = download(&c, url(&format!("{EXP}/{login_task}/download")), &admin).await;
    assert_eq!(st, 200, "已完成的任务能下载");
    assert_eq!(async_csv, sync_csv(&c, url("/login-logs/export?username=ex4_ghost"), &admin).await,
        "登录记录：异步导出 == 同步导出");
    let (st, async_audit) = download(&c, url(&format!("{EXP}/{audit_task}/download")), &admin).await;
    assert_eq!(st, 200);
    assert_eq!(async_audit, sync_csv(&c, url("/audit-logs/export?module=dept"), &admin).await,
        "操作日志：异步导出 == 同步导出");

    // 附件头：客户端要按它命名落盘
    let r = c.get(url(&format!("{EXP}/{login_task}/download"))).bearer_auth(&admin).send().await.unwrap();
    let cd = r.headers()["content-disposition"].to_str().unwrap().to_string();
    assert!(cd.contains("attachment") && cd.contains(".csv"), "附件头: {cd}");

    // ── 越权：别人的任务看不了、下不了、删不了 ──
    let (st, v) = download(&c, url(&format!("{EXP}/{login_task}/download")), &center_tok).await;
    assert_eq!(st, 403, "别人的导出不能下: {v:?}");
    let (st, v) = call(&c, Method::DELETE, url(&format!("{EXP}/{login_task}")), Some(&center_tok), None).await;
    assert_eq!(st, 403, "别人的导出不能删: {v}");
    let (st, v) = call(&c, Method::GET, url(&format!("{EXP}?size=50")), Some(&center_tok), None).await;
    assert_eq!(st, 200);
    assert_eq!(v["data"]["total"], 0, "别人的任务不出现在自己列表里: {v}");

    // ── 补上登录记录查看码后：自己发起、自己下载、自己删除 ──
    let ll = menu_id(&menus["data"], "system:loginlog:list").expect("登录记录菜单");
    let (st, v) = call(&c, Method::PUT, url(&format!("/roles/{rid}/menus")), Some(&admin),
        Some(json!({"menu_ids": [center, rm, ll]}))).await;
    assert_eq!(st, 200, "{v}");
    let (st, mine) = call(&c, Method::POST, url(EXP), Some(&center_tok),
        Some(json!({"kind": "login-log", "params": {"username": "ex4_ghost"}}))).await;
    assert_eq!(st, 200, "{mine}");
    let mine_id = common::as_id(&mine["data"]["id"]);
    common::trigger(&c, &base, &run_id, &admin).await;
    let (status, _, msg) = wait_task(&pool, common::dec_id(&mine_id)).await;
    assert_eq!(status, 2, "后台跑完: {msg}");

    let (st, his) = download(&c, url(&format!("{EXP}/{mine_id}/download")), &center_tok).await;
    assert_eq!(st, 200, "自己的导出能下");
    assert_eq!(his, sync_csv(&c, url("/login-logs/export?username=ex4_ghost"), &center_tok).await,
        "非超管：异步 == 同步（数据范围是同一条规则）");

    // 删除自己的：记录没了，文件也没了
    let file_path: String = sqlx::query_scalar("SELECT file_path FROM export_task WHERE id = ?")
        .bind(common::dec_id(&mine_id))
        .fetch_one(&pool)
        .await
        .expect("任务行里应有文件路径");
    assert!(std::path::Path::new(&file_path).exists(), "跑完的任务文件在盘上: {file_path}");
    let (st, v) = call(&c, Method::DELETE, url(&format!("{EXP}/{mine_id}")), Some(&center_tok), None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["data"]["deleted"], 1, "{v}");
    assert!(!std::path::Path::new(&file_path).exists(), "删任务要连文件一起删: {file_path}");

    // ── 过期清理：改成已过期 → 跑清理任务 → 记录与文件都没了，未过期的还在 ──
    let audit_path: String = sqlx::query_scalar("SELECT file_path FROM export_task WHERE id = ?")
        .bind(common::dec_id(&audit_task))
        .fetch_one(&pool)
        .await
        .unwrap();
    // 时戳在 Rust 侧给定值（不依赖 MySQL NOW()：sqlx 会话时区是 UTC，本机库是 +8）
    let past = chrono::NaiveDateTime::parse_from_str("2020-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    sqlx::query("UPDATE export_task SET expire_at = ? WHERE id = ?")
        .bind(past)
        .bind(common::dec_id(&audit_task))
        .execute(&pool)
        .await
        .unwrap();
    let clean_id = job_id(&c, &base, &admin, "export_clean").await;
    let before = common::last_log_id(&pool, "export_clean").await;
    common::trigger(&c, &base, &clean_id, &admin).await;
    let (status, msg, _ms) = common::wait_job_log(&pool, "export_clean", before).await;
    assert_eq!(status, 1, "清理任务必须成功: {msg}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_task WHERE id = ?")
        .bind(common::dec_id(&audit_task))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "过期任务连记录一起清");
    assert!(!std::path::Path::new(&audit_path).exists(), "过期文件也删了: {audit_path}");
    let (st, list) = call(&c, Method::GET, url(&format!("{EXP}?size=50")), Some(&admin), None).await;
    assert_eq!(st, 200);
    assert_eq!(list["data"]["total"], 1, "只清过期的，未过期的还在: {list}");

    // ── 审计留痕：发起/删除导出都有记录 ──
    let (st, v) = call(&c, Method::GET, url("/audit-logs?module=export&size=50"), Some(&admin), None).await;
    assert_eq!(st, 200, "{v}");
    let rows = v["data"]["list"].as_array().unwrap();
    assert!(
        rows.iter().any(|r| r["action"].as_str().unwrap_or_default().contains("导出")),
        "导出动作要留痕: {v}"
    );

    // ── 认领排他（回归）：被别的执行者**刚认领**的任务，再跑一次 export_run 必须 0 命中 ──
    // 老实现 `WHERE id=? AND status IN (0,1)`：行被推成 1 之后第二个执行者照样命中同一个
    // 谓词 → 同一任务两个执行者（双写 .part、双 finish 互相覆盖）。
    let (st, t_dup) = call(&c, Method::POST, url(EXP), Some(&admin),
        Some(json!({"kind": "login-log", "params": {"username": "ex4_ghost"}}))).await;
    assert_eq!(st, 200, "{t_dup}");
    let dup = common::dec_id(&common::as_id(&t_dup["data"]["id"]));
    // 摆出「另一个执行者刚认领」的现场：status=1 + started_at 新鲜。
    // 时戳在 Rust 侧算（与 src 的 util::now 同一个钟；MySQL NOW() 会话时区是 UTC，偏 8 小时）
    let claimed_at = chrono::Local::now().naive_local();
    sqlx::query("UPDATE export_task SET status = 1, started_at = ? WHERE id = ?")
        .bind(claimed_at)
        .bind(dup)
        .execute(&pool)
        .await
        .unwrap();
    let before = common::last_log_id(&pool, "export_run").await;
    common::trigger(&c, &base, &run_id, &admin).await;
    let (status, msg, _ms) = common::wait_job_log(&pool, "export_run", before).await;
    assert_eq!(status, 1, "{msg}");
    assert_eq!(msg, "导出任务 0 个（成功 0 / 失败 0）",
        "执行中（未超时）的任务第二次认领必须 0 行: {msg}");
    let (started, status, msg): (chrono::NaiveDateTime, i8, String) =
        sqlx::query_as("SELECT started_at, status, msg FROM export_task WHERE id = ?")
            .bind(dup)
            .fetch_one(&pool)
            .await
            .unwrap();
    // DATETIME 列不带小数秒：比到秒（真被刷新的话 status/msg 那几条断言也会炸）
    let at = |t: &chrono::NaiveDateTime| t.format("%Y-%m-%d %H:%M:%S").to_string();
    assert_eq!((at(&started), status), (at(&claimed_at), 1),
        "行要原封不动：started_at 没被刷新、状态没被推走");
    assert!(msg.is_empty(), "第二个执行者没跑它: {msg:?}");

    // ── 认领**语义**必须排他（回归）──
    // 上面那半钉的是「选中」语义（执行中的行不该被再选中）；这半钉库级语义：
    // 同一行连续认领两次，第二次必须 0 行。老实现 `WHERE id=? AND status IN (0, 1)`
    // 在第一次把行推成 1 之后照样命中（1 ∈ (0,1)），第二个执行者拿着「还是 0」的旧
    // 快照再认领就会真的跑第二遍。这里的两条语句是 crate::export::claim_sql 的**副本**
    //（bin crate 集成测试不能 use 它的代码，与 common 里重写 hashids 同一个理由）——
    // 副本钉不住 src（谓词回退时它照样绿），所以谓词本身的排他性由 export.rs 里
    // `claim_predicates_are_exclusive` 那条无库单测钉；这段负责证明「按这谓词跑，第二次真是 0 行」。
    let (st, t_claim) = call(&c, Method::POST, url(EXP), Some(&admin),
        Some(json!({"kind": "login-log", "params": {"username": "ex4_ghost"}}))).await;
    assert_eq!(st, 200, "{t_claim}");
    let cid = common::dec_id(&common::as_id(&t_claim["data"]["id"]));
    let t1 = chrono::Local::now().naive_local();
    let n1: u64 = sqlx::query("UPDATE export_task SET status = 1, started_at = ? WHERE id = ? AND status = 0")
        .bind(t1).bind(cid).execute(&pool).await.unwrap().rows_affected();
    assert_eq!(n1, 1, "待处理 → 第一次认领命中");
    let n2: u64 = sqlx::query("UPDATE export_task SET status = 1, started_at = ? WHERE id = ? AND status = 0")
        .bind(t1).bind(cid).execute(&pool).await.unwrap().rows_affected();
    assert_eq!(n2, 0, "第二个执行者（旧快照仍以为它是待处理）必须认领不到");
    // 超时认领同理：第一次认领会刷新 started_at，第二个执行者按老时戳再认领必须 0 行
    let old_at = chrono::Local::now().naive_local() - chrono::Duration::seconds(601);
    let stale_line = chrono::Local::now().naive_local() - chrono::Duration::seconds(600);
    sqlx::query("UPDATE export_task SET started_at = ? WHERE id = ?")
        .bind(old_at).bind(cid).execute(&pool).await.unwrap();
    let claim_stale = "UPDATE export_task SET status = 1, started_at = ? WHERE id = ? AND status = 1 AND started_at < ?";
    let s1: u64 = sqlx::query(claim_stale)
        .bind(t1).bind(cid).bind(stale_line).execute(&pool).await.unwrap().rows_affected();
    assert_eq!(s1, 1, "卡住的行第一次认领命中（恢复机制还在）");
    let s2: u64 = sqlx::query(claim_stale)
        .bind(t1).bind(cid).bind(stale_line).execute(&pool).await.unwrap().rows_affected();
    assert_eq!(s2, 0, "started_at 已被刷新，第二个执行者认领不到");
    // 不想让这一行留在库里影响后面的断言：跑掉它（status=1 + 新鲜时戳 = 认领中，无害）
    let _ = t_claim;

    // 反面：认领**超时**（进程崩过留下的行）反而要能重跑，恢复机制不能被顺手改掉
    let long_ago = chrono::Local::now().naive_local() - chrono::Duration::seconds(601);
    sqlx::query("UPDATE export_task SET started_at = ? WHERE id = ?")
        .bind(long_ago)
        .bind(dup)
        .execute(&pool)
        .await
        .unwrap();
    common::trigger(&c, &base, &run_id, &admin).await;
    let (status, _, msg) = wait_task(&pool, dup).await;
    assert_eq!(status, 2, "超时的认领要能重跑: {msg}");
    let path: String = sqlx::query_scalar("SELECT file_path FROM export_task WHERE id = ?")
        .bind(dup)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(std::path::Path::new(&path).exists(), "重跑要真出文件: {path}");
    // 反面负控：这一幕的 .part 不留孤儿（后台执行跑完就改名了）
    let part = format!("{path}.part");
    assert!(!std::path::Path::new(&part).exists(), ".part 要收尾干净: {part}");

    // ── M1 回归：**失败**路径也要清 .part ──
    // 造一个跑到一半才失败的任务：用同名**目录**占住最终文件名，`rename(part, path)` 必失败。
    // 失败点在 .part 已经建好、表头 + 数据全写完、只差改名那一步 —— 正是 M1 要管的窗口：
    // 这里不清，磁盘上就留一份看起来像成果的 .csv.part。
    let (st, t_bad) = call(&c, Method::POST, url(EXP), Some(&admin),
        Some(json!({"kind": "login-log"}))).await;
    assert_eq!(st, 200, "建失败任务: {t_bad}");
    let bad_id = common::dec_id(&common::as_id(&t_bad["data"]["id"]));
    // upload_dir 是 cwd 相对的（`target/test-uploads`），与执行器拼路径的口径一致
    let bad_dst = format!("target/test-uploads/exports/{bad_id}.csv");
    std::fs::create_dir_all(&bad_dst).expect("占位目录");
    let before = common::last_log_id(&pool, "export_run").await;
    common::trigger(&c, &base, &run_id, &admin).await;
    let (jstatus, jmsg, _) = common::wait_job_log(&pool, "export_run", before).await;
    assert_eq!(jstatus, 1, "执行器自身不失败，失败的是那一个任务: {jmsg}");
    let (status, _, msg) = wait_task(&pool, bad_id).await;
    assert_eq!(status, 3, "改名失败要让任务变失败: {msg}");
    assert!(msg.contains("改名失败"), "失败原因要落库给界面看: {msg}");
    let part = format!("{bad_dst}.part");
    assert!(!std::path::Path::new(&part).exists(), "失败路径的 .part 必须清掉: {part}");
    std::fs::remove_dir_all(&bad_dst).expect("收掉占位目录");

    // ── M2 回归：**待处理**的任务过期也不能被清掉 ──
    // 老 clean() 只按 expire_at 筛：待处理的行（文件都还没生成）会被连记录一起删，
    // 用户的任务凭空消失。这里造一个「已过期但还没跑」的行，跑完清理后它必须还在。
    // （放最后：这一行得留着不被 export_run 顺手跑掉，前面那些「导出任务 0 个」的
    // 断言才不会被它打乱。）
    let (st, t_pend) = call(&c, Method::POST, url(EXP), Some(&admin),
        Some(json!({"kind": "login-log"}))).await;
    assert_eq!(st, 200, "建待处理任务: {t_pend}");
    let pend_id = common::dec_id(&common::as_id(&t_pend["data"]["id"]));
    sqlx::query("UPDATE export_task SET expire_at = ? WHERE id = ?")
        .bind(past)
        .bind(pend_id)
        .execute(&pool)
        .await
        .unwrap();
    let before = common::last_log_id(&pool, "export_clean").await;
    common::trigger(&c, &base, &clean_id, &admin).await;
    let (status, msg, _ms) = common::wait_job_log(&pool, "export_clean", before).await;
    assert_eq!(status, 1, "{msg}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_task WHERE id = ?")
        .bind(pend_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "待处理的任务过期也不能清（文件还没生成，删了等于任务消失）");

    // ── N5 回归：行里 file_path 为空也要删掉按约定落下的文件 ──
    // 现场：改名成功、回写 file_path 之前进程死掉 —— 行里路径空着，文件已经叫 {id}.csv 了。
    // 只信行里的 path 的话，这行被清掉时文件会永久留在磁盘上（再没人知道它叫什么）。
    let (st, t5) = call(&c, Method::POST, url(EXP), Some(&admin),
        Some(json!({"kind": "login-log"}))).await;
    assert_eq!(st, 200, "建用于 N5 的任务: {t5}");
    let id5 = common::dec_id(&common::as_id(&t5["data"]["id"]));
    let before = common::last_log_id(&pool, "export_run").await;
    common::trigger(&c, &base, &run_id, &admin).await;
    let (status, _, msg) = wait_task(&pool, id5).await;
    assert_eq!(status, 2, "先正常跑完一份: {msg}");
    let path5 = format!("target/test-uploads/exports/{id5}.csv");
    assert!(std::path::Path::new(&path5).exists(), "文件按约定落下: {path5}");
    sqlx::query("UPDATE export_task SET file_path = '', expire_at = ? WHERE id = ?")
        .bind(past)
        .bind(id5)
        .execute(&pool)
        .await
        .unwrap();
    let before = common::last_log_id(&pool, "export_clean").await;
    common::trigger(&c, &base, &clean_id, &admin).await;
    let (status, msg, _ms) = common::wait_job_log(&pool, "export_clean", before).await;
    assert_eq!(status, 1, "{msg}");
    assert!(!std::path::Path::new(&path5).exists(), "路径空也要按 {{id}}.csv 兜底删掉: {path5}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_task WHERE id = ?")
        .bind(id5)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "记录也清了");
}
