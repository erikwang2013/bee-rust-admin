// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! E2-7（M7）：删公告时连带清 `attachment` 行（同事务），磁盘文件**不删** ——
//! 交给 `attach_orphan_clean` 按「无行引用」回收（`api_attach_test.rs` 已覆盖回收侧）。
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

/// 建一条草稿公告（status=0：不发 `notice.published`，不惊动 webhook/邮件）。
async fn new_notice(c: &reqwest::Client, base: &str, token: &str, title: &str) -> i64 {
    let (st, v) = call(
        c,
        Method::POST,
        format!("{base}/api/v1/notices"),
        token,
        Some(json!({ "title": title, "content": "正文", "status": 0 })),
    )
    .await;
    assert_eq!(st, 200, "建公告 {title}: {v}");
    common::dec_id(v["data"]["id"].as_str().expect("公告 id"))
}

/// 落盘文件与 `stored_path` 的形状一致：`attach_<子目录>_<文件名>`。
fn attach_file(saved_path: &str) -> std::path::PathBuf {
    let mut parts = saved_path.splitn(3, '_');
    let (_, subdir, name) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
    std::path::Path::new("target/test-uploads/attach").join(subdir).join(name)
}

#[allow(clippy::too_many_arguments)]
async fn insert_attachment(
    pool: &MySqlPool,
    admin_id: i64,
    stored_path: &str,
    entity_type: &str,
    entity_id: i64,
    sort: i32,
) {
    let now = chrono::Local::now().naive_local().format("%Y-%m-%d %H:%M:%S").to_string();
    sqlx::query(
        "INSERT INTO attachment \
         (id, name, ext, mime, size, sha256, stored_path, admin_id, entity_type, entity_id, sort, created_at, updated_at) \
         VALUES (?, ?, 'png', 'image/png', 6, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(common::new_row_id())
    .bind(stored_path)
    .bind(format!("{:064x}", sort)) // 测试不校验哈希，只要非空且不撞
    .bind(stored_path)
    .bind(admin_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(sort)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
}

async fn count(pool: &MySqlPool, sql: &str, binds: &[i64]) -> i64 {
    let mut q = sqlx::query_scalar::<_, i64>(sql);
    for b in binds {
        q = q.bind(*b);
    }
    q.fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn notice_delete_cascades_attachment_rows_and_keeps_files() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let api = |p: &str| format!("{base}/api/v1{p}");
    let c = reqwest::Client::new();
    let token = common::login(&base, "admin", "admin123").await.expect("登录失败");
    let pool = MySqlPool::connect(&dsn).await.unwrap();
    let admin_id: i64 = sqlx::query_scalar("SELECT id FROM admin ORDER BY id LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    // ── 1. 两条公告，各自挂附件；另有一条未绑定附件（entity_type 为空）作负控
    let gone = new_notice(&c, &base, &token, "要删的公告").await;
    let kept = new_notice(&c, &base, &token, "留着的公告").await;
    let paths = ["attach_noticedel_gone-1.png", "attach_noticedel_gone-2.png", "attach_noticedel_kept.png"];
    for p in paths {
        let f = attach_file(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"png!!").unwrap();
    }
    insert_attachment(&pool, admin_id, paths[0], "notice", gone, 1).await;
    insert_attachment(&pool, admin_id, paths[1], "notice", gone, 2).await;
    insert_attachment(&pool, admin_id, paths[2], "notice", kept, 1).await;
    insert_attachment(&pool, admin_id, "attach_noticedel_loose.png", "", 0, 0).await;
    let now = chrono::Local::now().naive_local().format("%Y-%m-%d %H:%M:%S").to_string();
    for n in [gone, kept] {
        sqlx::query("INSERT INTO notice_read (notice_id, admin_id, read_at) VALUES (?, ?, ?)")
            .bind(n)
            .bind(admin_id)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();
    }

    // 删除前的状态不是空的 —— 否则删完的 0 是假绿
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM attachment WHERE entity_type = 'notice' AND entity_id = ?", &[gone]).await, 2);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM attachment WHERE entity_type = 'notice' AND entity_id = ?", &[kept]).await, 1);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM attachment WHERE entity_type = '' AND entity_id = 0", &[]).await, 1);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM notice_read WHERE notice_id = ?", &[gone]).await, 1);

    // ── 2. 删公告 → 200
    let (st, v) = call(&c, Method::DELETE, api(&format!("/notices/{}", common::enc_id(gone as u64))), &token, None).await;
    assert_eq!(st, 200, "删公告: {v}");

    // ── 3. 公告行、已读行、配图附件行全部没了（同一事务一起提交）
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM notice WHERE id = ?", &[gone]).await, 0, "公告行没了");
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM notice_read WHERE notice_id = ?", &[gone]).await, 0, "已读记录跟着删");
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM attachment WHERE entity_type = 'notice' AND entity_id = ?", &[gone]).await,
        0,
        "配图附件行跟着删"
    );

    // ── 4. 负控：别的公告的附件、未绑定附件、别的公告的一律不动
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM notice WHERE id = ?", &[kept]).await, 1, "别的公告还在");
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM notice_read WHERE notice_id = ?", &[kept]).await, 1, "别的公告的已读还在");
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM attachment WHERE entity_type = 'notice' AND entity_id = ?", &[kept]).await,
        1,
        "别的公告的配图不动"
    );
    let loose: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachment WHERE stored_path = 'attach_noticedel_loose.png'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(loose, 1, "未绑定的附件行不动");

    // ── 5. 文件不删：留着给 attach_orphan_clean 按「无行引用」收（不在这里碰磁盘）
    assert!(attach_file(paths[0]).exists(), "删公告不删文件（交给孤儿清理）: {}", paths[0]);
    assert!(attach_file(paths[1]).exists(), "同上: {}", paths[1]);
    assert!(attach_file(paths[2]).exists(), "别人的配图文件更不该动: {}", paths[2]);

    // ── 6. 再删一次 → 404（行确实没了）
    let (st, v) = call(&c, Method::DELETE, api(&format!("/notices/{}", common::enc_id(gone as u64))), &token, None).await;
    assert_eq!(st, 404, "重复删返回 404: {v}");

    let _ = std::fs::remove_dir_all("target/test-uploads/attach/noticedel");
}
