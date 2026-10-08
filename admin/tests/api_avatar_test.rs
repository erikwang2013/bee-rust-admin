// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 阶段 4b：头像分片上传（aetherupload-rust 1.0.3）的后端契约。
//! 契约见 `docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md`「阶段 4b 契约 · 头像分片上传」。
//!
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN；上传根是
//! `conf/app.conf.test` 里的 `target/test-uploads`（服务进程的 cwd 与本测试相同）。
mod common;

use base64::Engine;
use reqwest::Method;
use serde_json::{Value, json};

/// 1x1 PNG（真实文件字节，非伪造魔数）。
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// 1x1 PNG 的字节（68 字节，够切成两片）。
fn png_bytes() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(PNG_B64).unwrap()
}

/// 一份「够真」的 JPEG：插件按魔数认类型，前三个字节对就过（64 字节，与 PNG 内容不同）。
fn jpeg_bytes() -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.extend_from_slice(&[0u8; 60]);
    bytes
}

/// 一份 >128KB（分片大小）的「JPEG」：用来走真·多片的路径。
fn big_jpeg() -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.extend_from_slice(&[0u8; 200 * 1024]);
    bytes
}

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

/// multipart 请求（分片上传的两个入口都是表单，字段名与前端逐字一致）。
async fn post_form(
    c: &reqwest::Client,
    url: String,
    token: Option<&str>,
    form: reqwest::multipart::Form,
) -> (u16, Value) {
    let mut rb = c.post(url).multipart(form);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    let r = rb.send().await.expect("请求发送失败");
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

/// preprocess 表单（`group` 固定 avatar；`locale` 让插件回中文文案）。
fn preprocess_form(name: &str, size: usize, hash: &str) -> reqwest::multipart::Form {
    reqwest::multipart::Form::new()
        .text("resource_name", name.to_string())
        .text("resource_size", size.to_string())
        .text("resource_hash", hash.to_string())
        .text("group", "avatar".to_string())
        .text("locale", "zh".to_string())
}

/// chunk 表单：一片的字节挂在 `resource_chunk`（文件名随便给，浏览器也带一个）。
fn chunk_form(
    temp_base: &str,
    ext: &str,
    subdir: &str,
    hash: &str,
    index: usize,
    total: usize,
    bytes: Vec<u8>,
) -> reqwest::multipart::Form {
    reqwest::multipart::Form::new()
        .text("chunk_total", total.to_string())
        .text("chunk_index", index.to_string())
        .text("resource_temp_basename", temp_base.to_string())
        .text("resource_ext", ext.to_string())
        .text("group_subdir", subdir.to_string())
        .text("resource_hash", hash.to_string())
        .text("group", "avatar".to_string())
        .text("locale", "zh".to_string())
        .part(
            "resource_chunk",
            reqwest::multipart::Part::bytes(bytes).file_name("blob"),
        )
}

/// 库里这位管理员的 avatar 列（savedPath 或空串）。
async fn db_avatar(pool: &sqlx::MySqlPool, id: i64) -> String {
    sqlx::query_scalar("SELECT avatar FROM admin WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 头像组目录下的成品路径：`{upload_dir}/avatar/{子目录}/{文件名}`。
fn saved_file(saved_path: &str) -> std::path::PathBuf {
    let mut parts = saved_path.splitn(3, '_');
    let (_, subdir, name) = (
        parts.next().unwrap(),
        parts.next().unwrap(),
        parts.next().unwrap(),
    );
    std::path::Path::new("target/test-uploads")
        .join("avatar")
        .join(subdir)
        .join(name)
}

#[tokio::test]
async fn v18_avatar_chunked_upload() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    // 头像落盘目录：测试前清干净（上一轮留下的文件会让「旧文件被删」这类断言失真）
    let _ = std::fs::remove_dir_all("target/test-uploads/avatar");

    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");
    let admin_id = common::my_id(&base, &admin).await;
    let admin_num = common::dec_id(&admin_id);

    // ── 上传口要求登录（不挂插件那套公开路由的理由）─────────────────
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/preprocess"),
        None,
        preprocess_form("avatar.png", 68, &aetherupload::md5_hex(&png_bytes())),
    )
    .await;
    assert_eq!(st, 401, "preprocess 必须登录: {v}");
    assert_eq!(v["err"], "auth.unauthorized", "未登录信封: {v}");

    // ── preprocess：契约字段一个不少 ────────────────────────────────
    let png = png_bytes();
    let png_hash = aetherupload::md5_hex(&png);
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/preprocess"),
        Some(&admin),
        preprocess_form("avatar.png", png.len(), &png_hash),
    )
    .await;
    assert_eq!(st, 200, "预处理: {v}");
    let data = &v["data"];
    assert_eq!(data["error"], 0, "插件 error 为 0 表示成功: {v}");
    assert_eq!(
        data["chunkSize"], 128 * 1024,
        "chunkSize 由服务端给（必须小于 512KB 上限，否则头像永远只有一片）: {v}"
    );
    assert_eq!(data["resourceExt"], "png", "扩展名来自原文件名: {v}");
    assert_eq!(
        data["savedPath"], "",
        "没命中秒传时 savedPath 是空串（新进程的秒传索引是空的：新库第一次上传不可能误命中）: {v}"
    );
    let subdir = data["groupSubDir"].as_str().unwrap().to_string();
    assert!(
        subdir.len() == 6 && subdir.bytes().all(|b| b.is_ascii_digit()),
        "默认按月分目录（202610 这种）: {v}"
    );
    let temp_base = data["resourceTempBaseName"].as_str().unwrap().to_string();
    assert!(!temp_base.is_empty(), "临时名要给回来: {v}");
    assert_eq!(db_avatar(&pool, admin_num).await, "", "没传完之前不动库");

    // ── 分两片传（手工切，不依赖 chunkSize）：末片才落库 ───────────
    let (first, second) = png.split_at(30);
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/chunk"),
        Some(&admin),
        chunk_form(&temp_base, "png", &subdir, &png_hash, 1, 2, first.to_vec()),
    )
    .await;
    assert_eq!(st, 200, "第一片: {v}");
    assert_eq!(v["data"]["error"], 0, "{v}");
    assert_eq!(v["data"]["savedPath"], "", "中间片没有成品路径: {v}");
    assert_eq!(db_avatar(&pool, admin_num).await, "", "中间片不动库");

    let (st, v) = post_form(
        &c,
        api("/avatar/upload/chunk"),
        Some(&admin),
        chunk_form(&temp_base, "png", &subdir, &png_hash, 2, 2, second.to_vec()),
    )
    .await;
    assert_eq!(st, 200, "末片: {v}");
    let saved = v["data"]["savedPath"].as_str().unwrap_or("").to_string();
    assert!(!saved.is_empty(), "末片返回成品路径: {v}");
    assert!(saved.ends_with(".png"), "文件名是内容 md5 + 扩展名: {saved}");
    assert_eq!(saved, format!("avatar_{subdir}_{png_hash}.png"), "savedPath 三段式: {v}");
    assert_eq!(db_avatar(&pool, admin_num).await, saved, "末片把 savedPath 写进库");
    assert_eq!(
        v["data"]["avatar"], format!("/api/v1/avatar/{admin_id}"),
        "响应额外带一个可直接用的地址，前端立即刷新用: {v}"
    );
    assert!(saved_file(&saved).exists(), "成品落盘: {}", saved_file(&saved).display());

    // ── 读取：公开、形状不变 ──────────────────────────────────────
    let r = c.get(api(&format!("/avatar/{admin_id}"))).send().await.unwrap();
    assert_eq!(r.status(), 200, "头像读取公开（<img> 带不了 token）");
    assert_eq!(r.headers()["content-type"], "image/png", "Content-Type 由扩展名决定");
    assert_eq!(r.bytes().await.unwrap().as_ref(), png.as_slice(), "读回字节与上传一致");

    // 列表 / 详情 / 个人资料里的 avatar 仍是 URL（列里存的是 savedPath，不能漏出去）
    for (path, pointer) in [
        ("/admins", "/data/list/0/avatar"),
        (format!("/admins/{admin_id}").as_str(), "/data/avatar"),
        ("/auth/profile", "/data/user/avatar"),
    ] {
        let (st, v) = call(&c, Method::GET, api(path), Some(&admin), None).await;
        assert_eq!(st, 200, "{path}: {v}");
        assert_eq!(
            v.pointer(pointer),
            Some(&json!(format!("/api/v1/avatar/{admin_id}"))),
            "{path} 的 avatar 必须是可用的 URL: {v}"
        );
    }

    // ── 秒传：同一张图重传，preprocess 直接给成品路径 ───────────────
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/preprocess"),
        Some(&admin),
        preprocess_form("avatar.png", png.len(), &png_hash),
    )
    .await;
    assert_eq!(st, 200, "重传预处理: {v}");
    assert_eq!(v["data"]["savedPath"], saved, "秒传命中返回既有成品: {v}");
    assert_eq!(v["data"]["avatar"], format!("/api/v1/avatar/{admin_id}"), "{v}");
    assert_eq!(db_avatar(&pool, admin_num).await, saved, "秒传也要把库写成这个值");
    assert!(saved_file(&saved).exists(), "秒传命中不能把自己的文件删掉（幂等）");

    // ── 换一张图（换成 jpg）：库更新、旧文件删掉 ────────────────────
    let jpeg = jpeg_bytes();
    let jpeg_hash = aetherupload::md5_hex(&jpeg);
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/preprocess"),
        Some(&admin),
        preprocess_form("avatar.jpg", jpeg.len(), &jpeg_hash),
    )
    .await;
    assert_eq!(st, 200, "jpg 预处理: {v}");
    assert_eq!(v["data"]["resourceExt"], "jpg", "{v}");
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/chunk"),
        Some(&admin),
        chunk_form(
            v["data"]["resourceTempBaseName"].as_str().unwrap(),
            "jpg",
            v["data"]["groupSubDir"].as_str().unwrap(),
            &jpeg_hash,
            1,
            1,
            jpeg.clone(),
        ),
    )
    .await;
    assert_eq!(st, 200, "jpg 单片上传: {v}");
    let saved_jpg = v["data"]["savedPath"].as_str().unwrap_or("").to_string();
    assert!(saved_jpg.ends_with(".jpg"), "{v}");
    assert_eq!(db_avatar(&pool, admin_num).await, saved_jpg, "库换成新图: {v}");
    assert!(
        !saved_file(&saved).exists(),
        "上一张头像的文件必须删掉（否则每换一次头像多留一个文件）"
    );
    assert!(saved_file(&saved_jpg).exists());
    let r = c.get(api(&format!("/avatar/{admin_id}"))).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "image/jpeg", "换图后立刻生效");
    assert_eq!(r.bytes().await.unwrap().as_ref(), jpeg.as_slice());

    // 没有分块的重复末片是幂等的：不会把刚写好的文件删掉
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/chunk"),
        Some(&admin),
        chunk_form(
            "whatever-temp",
            "jpg",
            "202610",
            &jpeg_hash,
            1,
            1,
            jpeg.clone(),
        ),
    )
    .await;
    assert_eq!(st, 200, "重发末片: {v}");
    assert!(saved_file(&saved_jpg).exists(), "重发末片不该删掉自己的文件: {v}");

    // ── A → B → A：删旧文件必须连带清秒传索引，否则 A 命中死链 ───────
    // 现在库里是 jpg、png 的文件刚被删。再传同一张 png 若还走秒传命中，拿到的就是那个
    // 已删的 savedPath（头像 404）—— 所以这里必须重新上传，而且文件要真写回盘上。
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/preprocess"),
        Some(&admin),
        preprocess_form("avatar.png", png.len(), &png_hash),
    )
    .await;
    assert_eq!(st, 200, "重新预处理: {v}");
    assert_eq!(
        v["data"]["savedPath"], "",
        "旧文件删了，索引也得清掉（否则秒传命中一个不存在的文件）: {v}"
    );
    let temp_base = v["data"]["resourceTempBaseName"].as_str().unwrap().to_string();
    let (first, second) = png.split_at(30);
    for (idx, part) in [(1usize, first), (2usize, second)] {
        let (st, v) = post_form(
            &c,
            api("/avatar/upload/chunk"),
            Some(&admin),
            chunk_form(&temp_base, "png", &subdir, &png_hash, idx, 2, part.to_vec()),
        )
        .await;
        assert_eq!(st, 200, "重传第 {idx} 片: {v}");
        if idx == 2 {
            assert_eq!(v["data"]["savedPath"], saved, "重新落盘到同一个成品名: {v}");
        }
    }
    assert_eq!(db_avatar(&pool, admin_num).await, saved, "A → B → A：库指回 png 的成品");
    assert!(saved_file(&saved).exists(), "重传后文件必须真的在盘上（不是指向被删的那份）");
    let r = c.get(api(&format!("/avatar/{admin_id}"))).send().await.unwrap();
    assert_eq!(r.status(), 200, "换回旧图后头像必须读得到（命中死链就会 404）");
    assert_eq!(r.bytes().await.unwrap().as_ref(), png.as_slice());

    // 上一张头像的文件**先不在盘上**再换头像：`delete_resource` 会在 `fs::remove_file`
    // 那步报 NotFound 提前返回，**碰都不碰秒传索引** —— 索引里那条死链就是下面要抓的。
    // （模拟文件被清理任务/运维删掉；这个状态是真实存在的）
    std::fs::remove_file(saved_file(&saved)).unwrap();

    // ── 真·多片：>128KB 的文件按服务端给的 chunkSize 切，末片才组装 ────
    // （上面那个 68 字节的 png 是手工切两片验协议；这条复刻前端的真实切片方式）
    let big = big_jpeg();
    let big_hash = aetherupload::md5_hex(&big);
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/preprocess"),
        Some(&admin),
        preprocess_form("photo.jpg", big.len(), &big_hash),
    )
    .await;
    assert_eq!(st, 200, "大图预处理: {v}");
    let chunk_size = v["data"]["chunkSize"].as_u64().unwrap() as usize;
    let temp_base = v["data"]["resourceTempBaseName"].as_str().unwrap().to_string();
    let big_subdir = v["data"]["groupSubDir"].as_str().unwrap().to_string();
    let total = big.len().div_ceil(chunk_size);
    assert!(total >= 2, "512KB 上限的头像至少要切 2 片，否则分片是结构性的死代码: {total} 片");
    for idx in 1..=total {
        let start = (idx - 1) * chunk_size;
        let end = (start + chunk_size).min(big.len());
        let (st, v) = post_form(
            &c,
            api("/avatar/upload/chunk"),
            Some(&admin),
            chunk_form(&temp_base, "jpg", &big_subdir, &big_hash, idx, total, big[start..end].to_vec()),
        )
        .await;
        assert_eq!(st, 200, "大图第 {idx}/{total} 片: {v}");
        let saved_now = v["data"]["savedPath"].as_str().unwrap_or("").to_string();
        if idx < total {
            assert_eq!(saved_now, "", "中间片没有成品路径: {v}");
            assert_ne!(db_avatar(&pool, admin_num).await, "", "上一张头像还在库里");
        } else {
            assert!(!saved_now.is_empty(), "末片组装完成: {v}");
            assert_eq!(db_avatar(&pool, admin_num).await, saved_now, "末片才换库里的值");
            assert!(saved_file(&saved_now).exists(), "成品落盘: {}", saved_file(&saved_now).display());
            let r = c.get(api(&format!("/avatar/{admin_id}"))).send().await.unwrap();
            assert_eq!(r.status(), 200);
            assert_eq!(r.bytes().await.unwrap().as_ref(), big.as_slice(), "整份内容一致（多片拼对了）");
        }
    }

    // ── 传回「刚才文件已经不在的那张 png」：必须是重新上传，不是秒传命中 ──
    // 命中的话返回的就是那条指向已删文件的 savedPath，头像直接 404。
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/preprocess"),
        Some(&admin),
        preprocess_form("avatar.png", png.len(), &png_hash),
    )
    .await;
    assert_eq!(st, 200, "预处理: {v}");
    assert_eq!(
        v["data"]["savedPath"], "",
        "删文件失败不等于索引可以留着（命中即死链）: {v}"
    );
    let temp_base = v["data"]["resourceTempBaseName"].as_str().unwrap().to_string();
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/chunk"),
        Some(&admin),
        chunk_form(&temp_base, "png", &subdir, &png_hash, 1, 1, png.clone()),
    )
    .await;
    assert_eq!(st, 200, "重新上传: {v}");
    assert_eq!(v["data"]["savedPath"], saved, "{v}");
    assert!(saved_file(&saved).exists(), "文件必须真的重新落盘（不是指向被删的那份）");
    assert_eq!(db_avatar(&pool, admin_num).await, saved);
    let r = c.get(api(&format!("/avatar/{admin_id}"))).send().await.unwrap();
    assert_eq!(r.status(), 200, "换回旧图后头像必须读得到");
    assert_eq!(r.bytes().await.unwrap().as_ref(), png.as_slice());

    // ── savedPath 指向的文件没了（被清理任务/手工删）→ 404，不是 500 ──
    sqlx::query("UPDATE admin SET avatar = ? WHERE id = ?")
        .bind(format!("avatar_202610_{}.png", "0123456789abcdef".repeat(4))) // 形态合法、文件不存在
        .bind(admin_num)
        .execute(&pool)
        .await
        .unwrap();
    let r = c.get(api(&format!("/avatar/{admin_id}"))).send().await.unwrap();
    assert_eq!(r.status(), 404, "savedPath 的文件不在 → 404（读文件那步不许 500）");

    // ── 存量兼容：老 URL 形态的值 → 老路径上的文件照样读得出来 ───────
    let legacy_name = format!("{admin_num}.png");
    let legacy_path = std::path::Path::new("target/test-uploads/avatar").join(&legacy_name);
    std::fs::write(&legacy_path, &png).unwrap();
    sqlx::query("UPDATE admin SET avatar = ? WHERE id = ?")
        .bind(format!("/api/v1/avatar/{admin_id}")) // 4b 之前列里存的就是这种 URL（无扩展名）
        .bind(admin_num)
        .execute(&pool)
        .await
        .unwrap();
    let r = c.get(api(&format!("/avatar/{admin_id}"))).send().await.unwrap();
    assert_eq!(r.status(), 200, "存量头像不迁移也要能显示");
    assert_eq!(r.headers()["content-type"], "image/png");
    assert_eq!(r.bytes().await.unwrap().as_ref(), png.as_slice());

    // ── 错误路径：插件的文案原样回给前端，且不带我们表里的 err 码（文案随 locale 变）──
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/preprocess"),
        Some(&admin),
        preprocess_form("avatar.gif", 68, &png_hash),
    )
    .await;
    assert_eq!(st, 400, "扩展名不在白名单（gif）: {v}");
    assert!(
        v["msg"].as_str().unwrap_or("").contains("无效的文件类型"),
        "locale=zh 时是插件的中文文案: {v}"
    );
    assert!(v.get("err").is_none(), "插件文案没有稳定错误码: {v}");

    let (st, v) = post_form(
        &c,
        api("/avatar/upload/preprocess"),
        Some(&admin),
        preprocess_form("big.png", 600 * 1024, &png_hash),
    )
    .await;
    assert_eq!(st, 400, "超过 512 KB: {v}");
    assert!(
        v["msg"].as_str().unwrap_or("").contains("无效的文件大小"),
        "超限文案来自插件: {v}"
    );

    // 单片的字节也不能比整张图还大（服务端自己的闸，在进插件之前拦下）
    let (st, v) = post_form(
        &c,
        api("/avatar/upload/chunk"),
        Some(&admin),
        chunk_form(
            "big-temp",
            "png",
            &subdir,
            &png_hash,
            1,
            1,
            vec![0u8; 600 * 1024],
        ),
    )
    .await;
    assert_eq!(st, 400, "单片超限: {v}");
    assert!(v["msg"].as_str().unwrap_or("").contains("512 KB"), "{v}");

    // 库里没有这个管理员 → 404（读取路径先查行）
    let r = c
        .get(api(&format!("/avatar/{}", common::enc_id(999))))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "查不到管理员 → 404");

    // 行在、但没头像 → 404（给个必然 404 的地址不如让前端显示占位）
    sqlx::query("UPDATE admin SET avatar = '' WHERE id = ?")
        .bind(admin_num)
        .execute(&pool)
        .await
        .unwrap();
    let r = c.get(api(&format!("/avatar/{admin_id}"))).send().await.unwrap();
    assert_eq!(r.status(), 404, "没有头像 → 404");

    let _ = std::fs::remove_dir_all("target/test-uploads/avatar");
}
