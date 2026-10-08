// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 头像：分片上传（aetherupload-rust 内核接线）+ 读取。
//!
//! 契约见 `docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md`
//! 「阶段 4b 契约 · 头像分片上传」——字段名与前端 `aetherupload` 那套逐字对接。
//!
//! 只复用插件的**内核**（`UploadController` + `FormData`），不挂它自带的 `routes()`：
//! 插件那四条路由是公开的（`preprocess` / `uploading` 都在内），而头像口要求登录 ——
//! 直接挂等于开一个无鉴权的写盘口（文件名是内容 hash 派生的，每次上传都落新文件）。
//! 所以插件依赖只开零依赖内核（`default-features = false`），路由与信封都是我们的。
//!
//! 落盘位置由插件定：`{upload_dir}/avatar/{按月的子目录}/{整份 md5}.{ext}`；
//! `admin.avatar` 列存的是它的 `savedPath`（`avatar_<子目录>_<md5>.<ext>`），
//! 对外 URL 仍是 `/api/v1/avatar/{hashids(id)}`（[`avatar_url`]）—— 前端拿到的形状不变。
//!
//! **存量头像不用迁移**：4b 之前的列值是 `/api/v1/avatar/xxx` 这种老 URL，读路径
//! （[`get_avatar`]）两种形态都认，回落到老的 `{upload_dir}/avatar/{id}.{ext}`。

use crate::auth::Auth;
use crate::config::AppConfig;
use crate::error::{ApiError, AppMultipart, AppPath, ok};
use crate::hid;
use crate::models::Admin;
use crate::state::AppState;
use crate::util::now;
use aetherupload::{
    ChunkBody, Config as UploadConfig, FormData, GroupConfig, MemoryInstantStore, Runtime,
    UploadController,
};
use axum::Json;
use axum::extract::{Multipart, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use bee_orm::Model;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 上传分组名。**不能含下划线**：插件的 `savedPath` 是 `group_subdir_name` 三段式，
/// 用 `_` 分隔，含下划线的分组名会让解码错位、该分组的资源永久 404（插件在
/// `Config::resolve_group` 里直接拒）。
pub(crate) const AVATAR_GROUP: &str = "avatar";

/// 头像的硬闸（字节）：与分组配置的 `resource_maxsize` 同源，前端会先压缩。
const AVATAR_MAX_BYTES: u64 = 512 * 1024;

/// 分片大小（字节）：**必须小于 [`AVATAR_MAX_BYTES`]**，否则分片路径永远走不到
/// （见契约里的那段说明）。128KB → 满配头像 4 片。
const AVATAR_CHUNK_BYTES: u64 = 128 * 1024;

/// 头像允许的扩展名：插件分组白名单 + 孤儿清理的白名单，一处定义两处用。
/// 插件按真实内容反查扩展名（`image/jpeg` 先命中表里的 `jpg`），所以 JPEG 写 "jpg" 即可。
pub(crate) const AVATAR_EXTS: [&str; 2] = ["png", "jpg"];

/// 按 `[app] upload_dir` 装配插件运行时。
///
/// `root_dir` 传 `upload_dir`（路径 + `base_path` 传空串）—— 插件在 `Runtime::new` 里做的是
/// `base_path.join(root_dir)`，这么传就等于把上传根钉死在 `[app] upload_dir`（相对路径
/// 仍相对进程 cwd，与 4b 之前的老实现一致）。
///
/// `instant_completion` 开秒传（同一张图重传直接命中，一个分块都不用传）：
/// 索引是进程内的内存表（`MemoryInstantStore`），重启即空 —— 只影响「重传」是否省流量，
/// 不影响正确性（没命中就走完整上传）。
///
/// 构造失败（如存储驱动不可用）返回 `Err`，由 `main` 拒绝启动：**别带着半吊子状态跑**
/// （与 `encrypt_key` 一个态度）。返回 `Arc`：运行时不带内部可变状态，构造一次共享给
/// 所有请求（`UploadController::new` 就是拿 `Arc` 的）。
pub fn build_runtime(cfg: &AppConfig) -> Result<Arc<Runtime>, String> {
    let config = UploadConfig {
        // 根目录 = 头像上传根（绝对/相对都行，见上）
        root_dir: cfg.upload_dir.clone(),
        instant_completion: true,
        // 分片大小必须**小于**头像上限（契约「chunk_size 必须小于头像上限」）：用插件
        // 默认的 1MB 的话 512KB 的头像永远只有一片，「分片」在上传路径上根本不触发。
        // 128KB → 满配头像 4 片、压缩过的头像 1–2 片。写死，不做配置项。
        chunk_size: AVATAR_CHUNK_BYTES,
        // 只留 avatar 分组：`Config::default()` 自带的 `file` 分组（100MB、20 多种扩展名）
        // 在这里没有用途，删掉它就是删掉一条「往别的目录写大文件」的路径。
        groups: BTreeMap::from([(
            AVATAR_GROUP.to_string(),
            GroupConfig {
                group_dir: AVATAR_GROUP.to_string(),
                resource_maxsize: AVATAR_MAX_BYTES,
                resource_extensions: AVATAR_EXTS.iter().map(|e| e.to_string()).collect(),
                event_before_upload_complete: false,
                event_upload_complete: false,
            },
        )]),
        ..UploadConfig::default()
    };

    let runtime = Runtime::new(config, "")
        .map_err(|e| format!("初始化头像上传运行时失败（[app] upload_dir = {}）: {e}", cfg.upload_dir))?;

    // 插件不会自己建这两个目录，缺了第一个上传就会失败（错误还只是「创建子文件夹失败」）：
    // - `avatar/`：头像组的落盘根（插件只建它下面的按月子目录，要求父目录已存在）
    // - `_header/`：分块断点（续传位置）的存放目录（插件直接 `create` 文件，不建目录）
    // 建不出来说明 upload_dir 不可写 —— 现在就说清楚，别等第一张头像上传时才发现。
    for dir in [runtime.upload_root().join(AVATAR_GROUP), runtime.upload_root().join("_header")] {
        std::fs::create_dir_all(&dir).map_err(|e| {
            format!("创建头像上传目录 {} 失败（[app] upload_dir = {}）: {e}", dir.display(), cfg.upload_dir)
        })?;
    }

    Ok(Arc::new(runtime.with_instant(Arc::new(MemoryInstantStore::new()))))
}

/// 对外的头像地址（短串与其它 id 一致）。**形状不变**是契约的一部分：前端缓存/回显里
/// 存的就是它，换上传流程不该换它。
pub fn avatar_url(id: i64) -> String {
    format!("/api/v1/avatar/{}", hid::enc(id))
}

/// 列表 / 详情 / 个人资料里的 `avatar` 字段：列里存的是 savedPath（内部地址），
/// 对外一律换成可用的 URL；没有头像仍是空串（前端据此显示占位，别给一个必然 404 的地址）。
pub fn public_avatar(stored: &str, id: i64) -> String {
    if stored.trim().is_empty() { String::new() } else { avatar_url(id) }
}

/// 4b 之前的落盘路径：`{upload_dir}/avatar/{id}.{ext}`（列里那时存的是老 URL）。
fn legacy_avatar_path(cfg: &AppConfig, id: i64, ext: &str) -> PathBuf {
    Path::new(&cfg.upload_dir).join("avatar").join(format!("{id}.{ext}"))
}

/// `POST /api/v1/avatar/upload/preprocess` —— 校验参数、判秒传、建临时文件。
///
/// **秒传命中时（`savedPath` 非空）这里就把头像落库**：拿到非空 `savedPath` 的前端会
/// 直接结束流程、不再发 chunk（协议如此），不在这里处理的话换头像会静默失效。
pub async fn preprocess(
    State(state): State<AppState>,
    auth: Auth,
    AppMultipart(mut multipart): AppMultipart,
) -> Result<Json<Value>, ApiError> {
    let form = read_form(&mut multipart).await?;
    let request = form.to_preprocess_request();
    let result = run_kernel(&state, move |runtime| {
        UploadController::new(runtime).preprocess(&request)
    })
    .await?;

    let Some(message) = result.error else {
        let done = !result.saved_path.is_empty();
        if done {
            finish(&state, &auth.admin, &result.saved_path).await?;
        }
        let mut data = plugin_json(&result.to_json())?;
        if done {
            // 秒传命中：把可直接用的地址一并给前端（与末片的 chunk 响应一致）
            data["avatar"] = json!(avatar_url(auth.admin.id));
        }
        return Ok(ok(data));
    };
    Err(ApiError::BadRequest(message))
}

/// `POST /api/v1/avatar/upload/chunk` —— 逐片追加；末片组装完成后头像落库。
pub async fn chunk(
    State(state): State<AppState>,
    auth: Auth,
    AppMultipart(mut multipart): AppMultipart,
) -> Result<Json<Value>, ApiError> {
    let form = read_form(&mut multipart).await?;
    let request = form.to_save_chunk_request();
    let result =
        run_kernel(&state, move |runtime| UploadController::new(runtime).save_chunk(&request)).await?;

    let Some(message) = result.error else {
        let mut data = plugin_json(&result.to_json())?;
        // `savedPath` 非空 = 组装完成（末片，或秒传命中后的幂等重发）；空串 = 中间片，库不动
        if !result.saved_path.is_empty() {
            finish(&state, &auth.admin, &result.saved_path).await?;
            data["avatar"] = json!(avatar_url(auth.admin.id));
        }
        return Ok(ok(data));
    };
    Err(ApiError::BadRequest(message))
}

/// 上传完成：把 `savedPath` 写进 `admin.avatar`，并删掉这位管理员**上一张**头像文件
/// （否则每换一次头像就多留一个文件，只涨不跌）。
///
/// 幂等：末片重发、秒传命中后前端又发了末片 —— 此时库里的值就是新的，`old == saved`，
/// 直接跳过删除，不会把刚写进去的文件删成 404。
async fn finish(state: &AppState, admin: &Admin, saved_path: &str) -> Result<(), ApiError> {
    let previous = admin.avatar.trim().to_string();

    let mut row = admin.clone();
    row.avatar = saved_path.to_string();
    row.updated_at = now();
    row.update(&state.db).await.map_err(ApiError::from)?;

    if previous.is_empty() || previous == saved_path {
        return Ok(());
    }

    if previous.starts_with('/') {
        // 老 URL 形态：文件在老的落盘路径上（`{upload_dir}/avatar/{id}.{ext}`），
        // 两种扩展名都试一遍（老实现的「换格式删旧文件」就是这么做的）
        for ext in AVATAR_EXTS {
            let path = legacy_avatar_path(&state.cfg, row.id, ext);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!("删旧头像 {} 失败: {e}", path.display()),
            }
        }
        return Ok(());
    }

    // savedPath 形态：先确认没有别的管理员还在引用同一份文件 —— 秒传（去重）意味着
    // 内容相同的头像共用同一个 savedPath，删之前不查引用会把别人的头像删成 404。
    // 本行此时已经改成新值，还匹配 `previous` 的都是别人。
    let others = Admin::query()
        .filter_eq("avatar", previous.clone())
        .map_err(ApiError::from)?
        .count(&state.db)
        .await
        .map_err(ApiError::from)?;
    if others > 0 {
        return Ok(());
    }

    // 交给插件删：文件 + 秒传索引一起清（索引留着的话，下次重传同一张图会
    // 「秒传命中」一个已经不存在的 savedPath → 头像 404）。
    if !state.aether.delete_resource(&previous) {
        tracing::debug!("删旧头像文件 {previous} 未成功（可能已被删或路径非法）");
    }
    // 但 `delete_resource` 只在**文件删成功**时才清索引：local 驱动就是 `remove_file`，
    // 文件早被别人删掉（手工删、清理任务删）时它报 NotFound 就提前返回，索引仍指向死文件。
    // 所以索引单独再清一次，不依赖上一步的结果。清索引失败只记日志 —— 它是缓存，
    // 不该让「换头像」失败；返回 false 基本只出现在 old 值形态非法时。
    if !state.aether.delete_instant_path(&previous) {
        tracing::warn!("清旧头像的秒传索引 {previous} 未成功（重传同一张图可能命中死链）");
    }
    Ok(())
}

/// 插件内核是同步的（末片要算整份 md5、分块要持排他锁追加）：丢进阻塞线程池，
/// 别占着 async worker（插件的取向说明里也是这么建议的）。
async fn run_kernel<T, F>(state: &AppState, job: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(Arc<Runtime>) -> T + Send + 'static,
{
    let runtime = state.aether.clone();
    tokio::task::spawn_blocking(move || job(runtime))
        .await
        .map_err(|e| ApiError::internal(format!("头像上传任务执行失败: {e}")))
}

/// 插件响应原文（`to_json()` 的 String）→ `Value`。
///
/// 字段名是它的 camelCase 契约（`chunkSize` / `savedPath` / `error`…），**原文透传**，
/// 我们只在末片额外补一个 `avatar` 键；不照着重拼一遍 —— 拼一遍就多一处会对不上的地方。
fn plugin_json(raw: &str) -> Result<Value, ApiError> {
    serde_json::from_str(raw)
        .map_err(|e| ApiError::internal(format!("解析上传响应失败: {e}（原文 {raw}）")))
}

/// multipart 表单 → 插件内核的 `FormData`。字段名与契约逐字一致
/// （`resource_name` / `resource_size` / `resource_hash` / `resource_chunk` …），
/// 由插件自己的 `to_preprocess_request()` / `to_save_chunk_request()` 翻译成内核入参。
async fn read_form(multipart: &mut Multipart) -> Result<FormData, ApiError> {
    let mut form = FormData::new();

    while let Some(field) = multipart.next_field().await.map_err(form_error)? {
        let Some(name) = field.name().map(str::to_string) else {
            continue;
        };
        if name == "resource_chunk" {
            let bytes = field.bytes().await.map_err(form_error)?;
            // 单独一片不可能比整张图还大：超了说明客户端没按服务端给的 chunkSize 切
            // （或有人在拿这个口塞别的东西）。真正的总大小闸在插件里（分组配置）。
            if bytes.len() as u64 > AVATAR_MAX_BYTES {
                return Err(ApiError::BadRequest("头像不能超过 512 KB".into()));
            }
            form.push_file(name, ChunkBody::Bytes(bytes.to_vec()));
        } else {
            let text = field.text().await.map_err(form_error)?;
            form.push_text(name, text);
        }
    }

    // 分组名服务端钉死：它决定落盘目录，不能让客户端挑（插件按 `group` 查分组配置）
    form.push_text("group", AVATAR_GROUP);
    Ok(form)
}

/// 表单读取阶段的错误同样回 `{code,msg,data}` 信封（`AppMultipart` 只管提取器那一步）。
fn form_error(e: axum::extract::multipart::MultipartError) -> ApiError {
    ApiError::BadRequest(format!("上传表单解析失败: {e}"))
}

/// `GET /api/v1/avatar/{hashids_id}`（公开 —— `<img>` 标签带不了 Authorization 头）。
///
/// 库里两种值形态都认：
/// - `/…` 开头 = 4b 之前的老 URL → 回落到老的 `{upload_dir}/avatar/{id}.{ext}`
///   （**存量头像不迁移也能显示**）；
/// - 其它非空值 = 插件的 `savedPath` → 按上传根解析后读文件。
///
/// 路径参数是 hashids 短串，解出数字 id 才查库；`savedPath` 的三段合法性与分组
/// 由插件校验（`Runtime::resource`），不存在路径穿越。读不到一律 404。
pub async fn get_avatar(
    State(state): State<AppState>,
    AppPath(id): AppPath<String>,
) -> Result<Response, ApiError> {
    let id = hid::dec(&id)?;

    let row = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    let stored = row.avatar.trim();
    if stored.is_empty() {
        return Err(ApiError::NotFound);
    }

    let path = if stored.starts_with('/') {
        AVATAR_EXTS
            .iter()
            .map(|ext| legacy_avatar_path(&state.cfg, id, ext))
            .find(|p| p.is_file())
    } else {
        state.aether.resource(stored).map(|r| r.path).filter(|p| p.is_file())
    };
    let Some(path) = path else {
        return Err(ApiError::NotFound);
    };

    let Some(content_type) = image_content_type(&path) else {
        // 头像组白名单之外的内容不该出现在这里（真出现了也不当文件发）
        return Err(ApiError::NotFound);
    };
    let bytes = std::fs::read(&path)
        .map_err(|e| ApiError::internal(format!("读头像文件 {} 失败: {e}", path.display())))?;

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, content_type.parse().expect("静态 MIME"));
    // no-cache：仍可缓存但每次回源校验，换头像后立刻生效
    headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    Ok((headers, bytes).into_response())
}

/// 按扩展名定 Content-Type；非图片返回 `None`（调用方按 404 处理）。
fn image_content_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只为 `build_runtime` 取 `upload_dir`；其余字段给能过配置校验的占位值。
    fn test_config() -> AppConfig {
        const SAMPLE: &str = r#"
[app]
name = bee-rust-admin-test
http_addr = 127.0.0.1:0
upload_dir = target/test-uploads
encrypt_key = base64:YmVlLWFkbWluLXRlc3Qta2V5LTMyLWJ5dGVzLW9rISE=

[db]
dsn = mysql://u:p@127.0.0.1:3306/db_test

[jwt]
secret = 0123456789012345678901234567890123
expire_hours = 1

[seed]
initial_admin_password = admin123
"#;
        let map = bee_rust::bee_config::ini::IniParser::parse(SAMPLE);
        AppConfig::from_ini(&map).expect("样例配置应当合法")
    }

    #[test]
    fn runtime_pins_the_upload_root_at_upload_dir() {
        let cfg = test_config();
        let runtime = build_runtime(&cfg).expect("构造应当成功");

        // `root_dir` = upload_dir + `base_path` = ""：上传根就是 [app] upload_dir
        assert_eq!(runtime.upload_root(), Path::new("target/test-uploads"));
        // 插件不会自己建这两个目录（缺了第一个上传就失败），构造时必须建好
        assert!(Path::new("target/test-uploads/avatar").is_dir());
        assert!(Path::new("target/test-uploads/_header").is_dir());

        // 分组配置：只此一个分组（默认的 `file` 被换掉），白名单与硬闸都对
        let snapshot = runtime.config().resolve_group(AVATAR_GROUP).expect("avatar 分组存在");
        assert_eq!(snapshot.group_dir, "avatar");
        assert_eq!(snapshot.resource_maxsize, AVATAR_MAX_BYTES);
        assert_eq!(snapshot.resource_extensions, vec!["png".to_string(), "jpg".to_string()]);
        assert!(snapshot.instant_completion, "秒传要开着（同一张图重传命中）");
        assert_eq!(
            snapshot.chunk_size, AVATAR_CHUNK_BYTES,
            "分片必须小于头像上限，否则分片路径永远走不到"
        );
        assert!(
            snapshot.chunk_size < AVATAR_MAX_BYTES,
            "契约：chunk_size < 头像上限"
        );
        assert!(
            runtime.config().resolve_group("file").is_err(),
            "默认的 file 分组不该留下（那是往别的目录写大文件的路径）"
        );
    }

    #[test]
    fn public_avatar_hides_the_saved_path() {
        // hid 的全局单例在单测里没人初始化（main 才调）：本用例要编短串，自己来一次。
        // OnceLock 只能初始化一次，别的用例先跑就复用它的盐（与 hid.rs 的 setup 同一套路）。
        let _ = crate::hid::init("hid-unit-test-salt", 8);
        // 存的是 savedPath（内部地址），对外的形状仍是 /api/v1/avatar/{短串}
        assert_eq!(
            public_avatar("avatar_202610_d41d8cd98f00b204e9800998ecf8427e.png", 7),
            avatar_url(7)
        );
        // 老 URL 形态（4b 之前的存量值）同样换成当前形状：两者本来就是同一个地址
        assert_eq!(public_avatar("/api/v1/avatar/abc", 7), avatar_url(7));
        // 没有头像：仍是空串（给个必然 404 的地址不如让前端显示占位）
        assert_eq!(public_avatar("", 7), "");
        assert_eq!(public_avatar("   ", 7), "");
    }

    #[test]
    fn image_content_type_follows_the_extension() {
        assert_eq!(image_content_type(Path::new("/x/a.png")), Some("image/png"));
        assert_eq!(image_content_type(Path::new("/x/a.JPG")), Some("image/jpeg"));
        assert_eq!(image_content_type(Path::new("/x/a.jpeg")), Some("image/jpeg"));
        assert_eq!(image_content_type(Path::new("/x/a.gif")), None);
        assert_eq!(image_content_type(Path::new("/x/a")), None);
    }
}
