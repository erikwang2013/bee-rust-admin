// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 修复 9：删管理员时一并清 `notice_read` 与该管理员的**未绑定**附件，
//! 但保留已绑到公告（公共配图）的附件，让孤儿清理能回收其余文件。
mod common;

use reqwest::Method;
use serde_json::{Value, json};
use sqlx::MySqlPool;

async fn call(
    c: &reqwest::Client,
    method: Method,
    url: String,
    token: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut rb = c.request(method, url).bearer_auth(token);
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    let r = rb.send().await.expect("请求发送失败");
    (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
}

async fn insert_attachment(
    pool: &MySqlPool,
    admin_id: i64,
    stored_path: &str,
    entity_type: &str,
    entity_id: i64,
) {
    let now = chrono::Local::now().naive_local().format("%Y-%m-%d %H:%M:%S").to_string();
    sqlx::query(
        "INSERT INTO attachment \
         (id, name, ext, mime, size, sha256, stored_path, admin_id, entity_type, entity_id, sort, created_at, updated_at) \
         VALUES (?, ?, 'png', 'image/png', 6, ?, ?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(common::new_row_id())
    .bind(stored_path)
    .bind(format!("{:064x}", common::new_row_id()))
    .bind(stored_path)
    .bind(admin_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
}

async fn count(pool: &MySqlPool, sql: &str, bind: i64) -> i64 {
    sqlx::query_scalar(sql).bind(bind).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn admin_delete_cleans_notice_reads_and_loose_attachments() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let c = reqwest::Client::new();
    let token = common::login(&base, "admin", "admin123").await.expect("超管登录");
    let pool = MySqlPool::connect(&dsn).await.unwrap();
    let admin_id = common::dec_id(&common::my_id(&base, &token).await);

    // 目标管理员 + 一条公告（公告配图属于公共内容，不随作者删除）
    let (st, v) = call(
        &c,
        Method::POST,
        api("/admins"),
        &token,
        Some(json!({"username": "gate_delete_target", "password": "target12345"})),
    )
    .await;
    assert_eq!(st, 200, "建目标管理员: {v}");
    let target = common::dec_id(&common::as_id(&v["data"]["id"]));

    let (st, v) = call(
        &c,
        Method::POST,
        api("/notices"),
        &token,
        Some(json!({"title": "公告配图保留", "content": "正文", "status": 0})),
    )
    .await;
    assert_eq!(st, 200, "建公告: {v}");
    let notice = common::dec_id(&common::as_id(&v["data"]["id"]));

    insert_attachment(&pool, target, "attach_gatedel_loose.png", "", 0).await;
    insert_attachment(&pool, target, "attach_gatedel_notice.png", "notice", notice).await;
    insert_attachment(&pool, admin_id, "attach_gatedel_admin.png", "", 0).await;
    let now = chrono::Local::now().naive_local().format("%Y-%m-%d %H:%M:%S").to_string();
    for aid in [target, admin_id] {
        sqlx::query("INSERT INTO notice_read (notice_id, admin_id, read_at) VALUES (?, ?, ?)")
            .bind(notice)
            .bind(aid)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();
    }

    // 删除前状态不是空的（否则后面的 0 是假绿）
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM notice_read WHERE admin_id = ?", target).await, 1);
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM attachment WHERE admin_id = ?", target).await,
        2
    );

    let (st, v) = call(
        &c,
        Method::DELETE,
        api(&format!("/admins/{}", common::enc_id(target as u64))),
        &token,
        None,
    )
    .await;
    assert_eq!(st, 200, "删管理员: {v}");

    // 管理员行、其已读记录、其未绑定附件都清掉
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM admin WHERE id = ?", target).await, 0);
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM notice_read WHERE admin_id = ?", target).await,
        0,
        "删管理员要一并清 notice_read，否则 notice_read 永远拦着公告删除"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM attachment WHERE admin_id = ? AND (entity_type = '' OR entity_id = 0)",
            target,
        )
        .await,
        0,
        "未绑定附件行要删掉，文件才能被 attach_orphan_clean 回收"
    );

    // 负控：公告及其配图、其他人的未绑定附件、其他人的已读记录都不动
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM notice WHERE id = ?", notice).await, 1);
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM attachment WHERE admin_id = ? AND entity_type = 'notice'",
            target,
        )
        .await,
        1,
        "绑在公告上的配图是公共内容，不能随作者删除"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM attachment WHERE admin_id = ? AND entity_type = ''",
            admin_id,
        )
        .await,
        1,
        "别人的附件不能误删"
    );
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM notice_read WHERE admin_id = ?", admin_id).await,
        1,
        "别人的已读记录不能误删"
    );
}
