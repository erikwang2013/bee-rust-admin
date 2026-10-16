// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 导入的两条边界补口（批次 E T1/T2）：
//!
//! - **T1 行数上限**：`MAX_ROWS = 5000` 是「解析阶段」的闸，超过就整份 400、零落库。
//!   边界要精确 —— 5001 行必拒、5000 行必须进门（进门后被逐行校验接管，坏行照常
//!   整批回滚）。这里两半都钉：5001 → 400；5000 行**故意全写坏**（上级部门不存在）→
//!   200 + 报告 5000 行 + created=0。5000 那半用坏行是刻意的：既证明没被行数闸拦下，
//!   又不用往库里真插 5000 行。
//! - **T2 编码闸走的是 HTTP 路径**：GBK 存的中文在 multipart 里就是一段非法 UTF-8 字节，
//!   接口必须 400 并提示换编码（`unit 测试钉的是 decode 函数`，这里钉的是「请求真的过它」）。
//!   同一份内容用正经 UTF-8 再传一次 → 200，对照排除「400 是内容问题不是编码问题」。
//!
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN。
mod common;

use serde_json::Value;

const IMP: &str = "/api/v1/imports";

/// 上传**原始字节**（T2 要传非法 UTF-8，不能走 `&str`）。字段名与前端逐字一致：`file`。
async fn post_csv_bytes(
    c: &reqwest::Client,
    url: String,
    token: &str,
    body: Vec<u8>,
) -> (u16, Value) {
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(body).file_name("import.csv"),
    );
    let r = c
        .post(url)
        .bearer_auth(token)
        .multipart(form)
        .send()
        .await
        .expect("上传请求失败");
    let status = r.status().as_u16();
    (status, r.json().await.unwrap_or(Value::Null))
}

/// 部门模板的 body：`rows` 行，上级部门故意写成不存在的名字（逐行必错）。
/// 行数闸只看行数、不看行内容，所以这正好拿来测「第 5000 行到底进没进门」。
fn dept_csv(rows: usize) -> String {
    let mut s = String::from("部门名称,上级部门,负责人,联系电话,排序,状态\n");
    for i in 1..=rows {
        s.push_str(&format!("压测行{i},不存在的部门,,,1,1\n"));
    }
    s
}

async fn dept_count(pool: &sqlx::MySqlPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM dept WHERE name LIKE '压测行%'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn import_row_cap_and_encoding_gate() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_srv, base) = common::start_server().await;
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");
    let c = reqwest::Client::new();
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();

    // ── T1a：5001 行 → 400，且一个字都没落库 ──
    let (st, v) = post_csv_bytes(
        &c,
        format!("{base}{IMP}?kind=dept"),
        &admin,
        dept_csv(5001).into_bytes(),
    )
    .await;
    assert_eq!(st, 400, "5001 行必须整份拒绝（行内容只是坏行的话本该是 200+报告）: {v}");
    let msg = v["msg"].as_str().unwrap_or_default();
    assert!(msg.contains("5000"), "报错要点名上限: {v}");
    assert_eq!(dept_count(&pool).await, 0, "被行数闸拦下 = 零落库");

    // ── T1b：正好 5000 行 → 进门（200 + 逐行报告；坏行整批回滚）──
    let (st, v) = post_csv_bytes(
        &c,
        format!("{base}{IMP}?kind=dept"),
        &admin,
        dept_csv(5000).into_bytes(),
    )
    .await;
    assert_eq!(st, 200, "5000 行在上限内，必须进到逐行校验（而不是被行数闸拒）: {v}");
    assert_eq!(v["data"]["created"], 0, "全坏行 → 整批回滚: {v}");
    assert_eq!(
        v["data"]["rows"].as_array().map(Vec::len),
        Some(5000),
        "报告要把 5000 行逐行列出来（少一行都说明中途被截断）: {v}"
    );
    assert_eq!(dept_count(&pool).await, 0, "回滚后一条都不许留下");

    // ── T2：GBK 字节 → 400 + 换编码提示；同一内容 UTF-8 → 200（对照）──
    // 两行的内容逐字相同、只是编码不同，把「400 是编码问题」钉死（内容本身完全合法）
    let mut gbk = String::from("部门名称,上级部门,负责人,联系电话,排序,状态\n").into_bytes();
    gbk.extend_from_slice(&[0xD6, 0xD0, 0xD7, 0xE9]); // GBK 的「中组」：不是合法 UTF-8
    gbk.extend_from_slice(",,,,10,1\n".as_bytes());
    let (st, v) = post_csv_bytes(&c, format!("{base}{IMP}?kind=dept"), &admin, gbk).await;
    assert_eq!(st, 400, "GBK 存的中文必须被编码闸拦下: {v}");
    let msg = v["msg"].as_str().unwrap_or_default();
    assert!(msg.contains("UTF-8"), "提示里要说清是编码问题（并给自救办法）: {v}");

    // 同样两行、正经 UTF-8：必须 200 —— 不然刚才那个 400 可能是内容而非编码导致的
    let utf8 = "部门名称,上级部门,负责人,联系电话,排序,状态\n中组,,,,10,1\n";
    let (st, v) = post_csv_bytes(
        &c,
        format!("{base}{IMP}?kind=dept"),
        &admin,
        utf8.as_bytes().to_vec(),
    )
    .await;
    assert_eq!(st, 200, "UTF-8 的同一份内容必须导入成功: {v}");
    assert_eq!(v["data"]["created"], 1, "对照行真的落库: {v}");
    assert_eq!(dept_count(&pool).await, 0, "「压测行」全被回滚、只有对照行落库");

    let tree = c
        .get(format!("{base}/api/v1/depts/tree"))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&tree).unwrap().contains("中组"),
        "对照行在树里: {tree}"
    );
}
