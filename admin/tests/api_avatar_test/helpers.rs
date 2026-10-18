// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 头像集成测试的本地构造（F3 拆分：主文件只留用例）。

use base64::Engine;
use reqwest::Method;
use serde_json::Value;

/// 1x1 PNG（真实文件字节，非伪造魔数）。
pub(super) const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// 1x1 PNG 的字节（68 字节，够切成两片）。
pub(super) fn png_bytes() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(PNG_B64).unwrap()
}

/// 一份「够真」的 JPEG：插件按魔数认类型，前三个字节对就过（64 字节，与 PNG 内容不同）。
pub(super) fn jpeg_bytes() -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.extend_from_slice(&[0u8; 60]);
    bytes
}

/// 一份 >128KB（分片大小）的「JPEG」：用来走真·多片的路径。
pub(super) fn big_jpeg() -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.extend_from_slice(&[0u8; 200 * 1024]);
    bytes
}

/// 统一请求：返回 (状态码, JSON body)。
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

/// multipart 请求（分片上传的两个入口都是表单，字段名与前端逐字一致）。
pub(super) async fn post_form(
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
pub(super) fn preprocess_form(name: &str, size: usize, hash: &str) -> reqwest::multipart::Form {
    reqwest::multipart::Form::new()
        .text("resource_name", name.to_string())
        .text("resource_size", size.to_string())
        .text("resource_hash", hash.to_string())
        .text("group", "avatar".to_string())
        .text("locale", "zh".to_string())
}

/// chunk 表单：一片的字节挂在 `resource_chunk`（文件名随便给，浏览器也带一个）。
pub(super) fn chunk_form(
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

/// 库里这位管理员的 avatar 列（saved_path 或空串）。
pub(super) async fn db_avatar(pool: &sqlx::MySqlPool, id: i64) -> String {
    sqlx::query_scalar("SELECT avatar FROM admin WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 头像组目录下的成品路径：`{upload_dir}/avatar/{子目录}/{文件名}`。
pub(super) fn saved_file(saved_path: &str) -> std::path::PathBuf {
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
