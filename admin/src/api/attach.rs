// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 附件中心（项 5）：分片上传 / 列表 / 下载 / 多态关联 / 删除。
//!
//! 上传复用 `api::avatar` 那套 aetherupload 内核接线（同一份 `[app] upload_dir` 下的
//! 另一个分组 `attach`，见 [`crate::api::avatar::build_runtime`]），差别只在闸门与落库：
//! 附件上限 10MB（`AVATAR_MAX_BYTES` 的 20 倍），落 `attachment` 行、留原文件名与 sha256。
//!
//! **下载鉴权（本模块的核心口径）**：
//! 1. 必须登录（不给公开路由 —— 头像那条公开路是给 `<img>` 的，附件不给）；
//! 2. 绑定了**已发布**公告的附件 = 公开：公告本身对所有登录用户可见，配图必须跟着可见；
//! 3. 其余一律按上传人的数据权限卡（与列表同一个口径 `datascope::ensure_admin_in_scope`）。
//!
//! 删除比下载更严：**公开规则不适用**（配图是公告的一部分，删附件得看数据权限），
//! 且要求 `system:attach:remove`。
use crate::api::{PagingExt, page_size};
use crate::api::avatar::{problem_to_api_error, run_kernel};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppJson, AppMultipart, AppPath, AppQuery, ok};
use crate::hid;
use crate::models::{Admin, Attachment, Notice};
use crate::state::AppState;
use crate::util::now;
use aetherupload::json::{save_chunk_ok_to_json, upload_ok_to_json};
use aetherupload::{ChunkBody, FormData, UploadController};
use axum::Json;
use axum::extract::{Multipart, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bee_orm::{Model, Value as DbValue};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

/// 上传分组名（落盘子目录）。与 `avatar` 一样**不能含下划线**：`saved_path` 是
/// `group_subdir_name` 三段式，含下划线的分组名会让解码错位。
pub(crate) const ATTACH_GROUP: &str = "attach";

/// 单文件硬闸（字节）：与分组配置的 `resource_maxsize` 同源，前端先拦一道。
pub(crate) const ATTACH_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// 允许的扩展名（分组白名单）。**两道都要过**：客户端声明的扩展名，以及插件按**真实
/// 内容**反查出的扩展名（`partial.check_mime_type`）。所以这里的名单得让两边都认得：
/// - 图片 / pdf / txt：魔数可识别，反查回同一族扩展名；
/// - docx / xlsx / pptx 是 zip 容器：内容判为 `application/zip` → 反查回 `zip`（在列），
///   客户端那份 `docx` 也在列 → 过；
/// - **旧版 doc/xls/ppt（OLE2）收不了**：内容判为 `application/octet-stream` → 反查回
///   `bin`，不在列。要收它就得放 `bin`（等于放行一切二进制），不做。
pub(crate) const ATTACH_EXTS: [&str; 13] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "pdf", "txt", "csv", "docx", "xlsx", "pptx",
    "zip",
];

/// 原始文件名的最大长度（列宽 255）：超了截断，别让 MySQL 报 1406 变成 500。
const NAME_MAX: usize = 255;

/// 允许关联的对象类型。**白名单**：多态关联的另一头要能查得到、判得出「公不公开」，
/// 放开任意字符串等于允许往库里写查不出来的幽灵关联。
pub(crate) const ENTITY_TYPES: [&str; 1] = ["notice"];

/// 关联排序的取值范围（同组配图顺序，前端按它排）。
const SORT_MAX: i32 = 999;

// ── 上传 ────────────────────────────────────────────────────

/// `POST /api/v1/attachments/upload/preprocess` —— 校验参数（类型/大小/配额）、判秒传、建临时文件。
#[apidoc::title("附件上传 · 预检")]
#[apidoc::desc("校验扩展名白名单（415）、单文件上限 10MB 与**每人总配额**（400 `attach.too_large` / `attach.quota_exceeded`）、判秒传、建临时文件。需要 `system:attach:add`")]
#[apidoc::url("/api/v1/attachments/upload/preprocess")]
#[apidoc::method("POST")]
#[apidoc::tag("附件")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "resource_name", ty = "string", required, desc = "原始文件名（按扩展名校验，见白名单）", mock = "report.pdf")]
#[apidoc::param(name = "resource_size", ty = "int", required, desc = "文件字节数，上限 10 MB", mock = "204800")]
#[apidoc::param(name = "name", ty = "string", required, desc = "原始文件名（落库与下载用，可与 resource_name 相同）", mock = "report.pdf")]
#[apidoc::param(name = "resource_hash", ty = "string", required, desc = "整文件 MD5（秒传索引键）", mock = "d41d8cd98f00b204e9800998ecf8427e")]
#[apidoc::not_debug]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("401")]
#[apidoc::response_status("403")]
#[apidoc::response_status("415")]
#[apidoc::returned(name = "data", ty = "object", desc = "插件响应：saved_path 非空 = 秒传命中（同时带 attachment 行）；否则带分片信息（chunk_size 等）")]
pub async fn preprocess(
    State(state): State<AppState>,
    auth: Auth,
    AppMultipart(mut multipart): AppMultipart,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:attach:add")?;
    let (form, name) = read_form(&mut multipart).await?;
    // 配额在预检处拦：分片都传上来了再拒，白费流量，文件也已落了一半
    check_quota(&state, &auth, &form).await?;

    let request = form.to_preprocess_request();
    let result =
        run_kernel(&state, move |runtime| UploadController::new(runtime).preprocess(&request))
            .await?;

    match result {
        Ok(parsed) => {
            let mut data = plugin_json(&upload_ok_to_json(&parsed))?;
            // 秒传命中：拿到非空 saved_path 的前端会直接结束流程，不在这次落库就等于没传
            if !parsed.saved_path.is_empty() {
                data["attachment"] = finish(&state, &auth.admin.id, &parsed.saved_path, &name).await?;
            }
            Ok(ok(data))
        }
        Err(problem) => Err(problem_to_api_error(problem)),
    }
}

/// `POST /api/v1/attachments/upload/chunk` —— 逐片追加；末片组装完成后落 `attachment` 行。
#[apidoc::title("附件上传 · 上传分片")]
#[apidoc::desc("multipart 逐片追加（128 KB/片）；末片组装完成后落库并回 attachment 行。失败按插件的真实状态码与稳定码返回（409 分块乱序时可带 `args.expected_index`，据它续传）。需要 `system:attach:add`")]
#[apidoc::url("/api/v1/attachments/upload/chunk")]
#[apidoc::method("POST")]
#[apidoc::tag("附件")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "resource_chunk", ty = "file", required, desc = "分片内容（单片不超过 preprocess 返回的 chunk_size，128 KB）")]
#[apidoc::param(name = "chunk_index", ty = "int", required, desc = "分片序号，从 1 开始", mock = "1")]
#[apidoc::param(name = "chunk_total", ty = "int", required, desc = "分片总数", mock = "3")]
#[apidoc::param(name = "resource_hash", ty = "string", required, desc = "整文件 MD5（与预检同一个值）")]
#[apidoc::not_debug]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("403")]
#[apidoc::response_status("409")]
#[apidoc::response_status("413")]
#[apidoc::response_status("415")]
#[apidoc::response_status("422")]
#[apidoc::returned(name = "data", ty = "object", desc = "插件响应；末片（saved_path 非空）额外带 attachment 行")]
pub async fn chunk(
    State(state): State<AppState>,
    auth: Auth,
    AppMultipart(mut multipart): AppMultipart,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:attach:add")?;
    let (form, name) = read_form(&mut multipart).await?;
    let request = form.to_save_chunk_request();
    let result =
        run_kernel(&state, move |runtime| UploadController::new(runtime).save_chunk(&request))
            .await?;

    match result {
        Ok(parsed) => {
            let mut data = plugin_json(&save_chunk_ok_to_json(&parsed))?;
            // `saved_path` 非空 = 组装完成（末片，或秒传命中后的幂等重发）；空串 = 中间片
            if !parsed.saved_path.is_empty() {
                data["attachment"] = finish(&state, &auth.admin.id, &parsed.saved_path, &name).await?;
            }
            Ok(ok(data))
        }
        Err(problem) => Err(problem_to_api_error(problem)),
    }
}

/// 上传完成：按落盘文件算 size/sha256，落一行 `attachment`（未绑定）。
/// 返回给前端的那一行的 JSON（与列表同形状）。
async fn finish(
    state: &AppState,
    admin_id: &i64,
    saved_path: &str,
    name: &str,
) -> Result<Value, ApiError> {
    let path = state
        .aether
        .resource(saved_path)
        .map(|r| r.path)
        .filter(|p| p.is_file())
        .ok_or_else(|| ApiError::internal(format!("上传成品读不到：{saved_path}")))?;

    // 整份读进来算摘要：附件上限 10MB，丢阻塞线程池（同步 IO + 哈希都别占 async worker）
    let (size, sha256) = tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&path)?;
        Ok::<_, std::io::Error>((bytes.len() as i64, sha256_hex(&bytes)))
    })
    .await
    .map_err(|e| ApiError::internal(format!("附件摘要任务执行失败: {e}")))?
    .map_err(|e| ApiError::internal(format!("读附件成品失败: {e}")))?;

    // 配额按**实测** size 复查：预检那道只看客户端声明的 resource_size，声明 1 字节即可绕过。
    // 这里拒掉时行不落库，成品文件成孤儿 —— `attach_orphan_clean` 会连秒传索引一起清掉。
    check_quota_bytes(state, *admin_id, size as u64).await?;

    let ext = Path::new(saved_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let row = Attachment {
        id: state.next_id()?,
        name: sanitize_name(name, saved_path),
        mime: aetherupload::mime::mime_for_extension(&ext, &[]),
        ext,
        size,
        sha256,
        stored_path: saved_path.to_string(),
        admin_id: *admin_id,
        entity_type: String::new(),
        entity_id: 0,
        sort: 0,
        created_at: now(),
        updated_at: now(),
    };
    row.insert(&state.db).await.map_err(ApiError::from)?;

    Ok(row_json(&row, &HashMap::new(), &HashMap::new()))
}

/// 原始文件名的清洗：去掉路径分隔符与控制字符（防「../」与终端转义混进文件头），
/// 压缩空白、限长；清完为空就退回落盘文件名（别给一个空名字）。
fn sanitize_name(name: &str, saved_path: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | '"' | '\'') { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return saved_path.rsplit('_').next().unwrap_or("attachment").to_string();
    }
    cleaned.chars().take(NAME_MAX).collect()
}

/// 十六进制 sha256。手写编码：仓库里没有 hex 依赖，为 20 行代码不值得加一个。
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// 单文件上限（`attach.too_large`）：自己拦一道，给稳定码与准确文案；插件那道
/// `payload_too_large`（413）当兜底。
fn too_large_err() -> ApiError {
    ApiError::Coded {
        status: StatusCode::BAD_REQUEST,
        err: "attach.too_large",
        msg: TOO_LARGE_MSG.to_string(),
        args: Value::Null,
    }
}

/// 与 `error::ERR_TABLE` 共用一份（改文案只改这里，否则稳定码会静默消失）。
pub(crate) const TOO_LARGE_MSG: &str = "附件不能超过 10 MB";

/// 每人**在用附件总量**的配额闸（`[attach] quota_mb`，0 = 不限）。
/// 口径是 `SUM(size)` 而不是「上传次数」：配额的意义是磁盘占用。
async fn check_quota(state: &AppState, auth: &Auth, form: &FormData) -> Result<(), ApiError> {
    let incoming: u64 = form
        .text("resource_size")
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    check_quota_bytes(state, auth.admin.id, incoming).await
}

/// 配额闸的唯一口径：`该管理员现有附件总量 + incoming > quota`（MB 折算后）就拒。
///
/// 两处调用，**入参的可信度不同**：预检拿的是客户端声明的 `resource_size`（要拦早，
/// 不然分片白传；但它不可信，声明 1 字节就能过），`finish` 拿的是落盘文件的**实测**
/// size（唯一拦得住的一道）。被 `finish` 拒掉的成品没人引用，交给 attach_orphan_clean。
async fn check_quota_bytes(state: &AppState, admin_id: i64, incoming: u64) -> Result<(), ApiError> {
    let quota_mb = state.cfg.attach_quota_mb;
    if quota_mb <= 0 {
        return Ok(());
    }
    let used = sum_size(state, admin_id).await?;
    let quota = quota_mb as u64 * 1024 * 1024;
    if used + incoming > quota {
        return Err(ApiError::Coded {
            status: StatusCode::BAD_REQUEST,
            err: "attach.quota_exceeded",
            msg: format!("附件总容量已超出配额（{quota_mb} MB），先删一些再传"),
            args: json!({ "quota_mb": quota_mb, "used": used, "incoming": incoming }),
        });
    }
    Ok(())
}

/// 某人的附件总量（字节）。ORM 没有聚合，走裸 SQL（表名与列名都是写死的常量，值走绑定）。
///
/// `CAST(… AS SIGNED)` 不能省：`SUM(bigint)` 在 MySQL 里是 **DECIMAL**，驱动把它当
/// 字符串取回来，`as_i64()` 拿不到值 —— 那样配额会静默恒为「已用 0」（踩过，整个
/// 配额闸形同虚设）。
async fn sum_size(state: &AppState, admin_id: i64) -> Result<u64, ApiError> {
    let rows = state
        .db
        .query(
            "SELECT CAST(COALESCE(SUM(size), 0) AS SIGNED) AS total FROM attachment \
             WHERE admin_id = ?",
            &[DbValue::from(admin_id)],
        )
        .await
        .map_err(ApiError::from)?;
    Ok(rows
        .first()
        .and_then(|r| r.get("total"))
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|n| n.max(0) as u64)))
        .unwrap_or(0))
}

// ── 列表 ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AttachQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    /// 原始文件名模糊
    pub name: Option<String>,
    /// 关联类型：`notice` 只看公告配图；`none` 只看未绑定
    pub entity_type: Option<String>,
    /// 只看某个对象的附件（公告编辑页要「这篇公告的配图」）；非法短串 400
    pub entity_id: Option<String>,
}

#[apidoc::title("附件列表")]
#[apidoc::desc("按文件名模糊 / 关联类型筛选 + 分页；受数据权限约束（只看得到自己范围内的上传人）。需要 `system:attach:list`")]
#[apidoc::url("/api/v1/attachments")]
#[apidoc::method("GET")]
#[apidoc::tag("附件")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100")]
#[apidoc::query(name = "name", ty = "string", desc = "原始文件名模糊匹配")]
#[apidoc::query(name = "entity_type", ty = "string", desc = "关联类型：notice / none（未绑定）")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "分页结果", children = [
    {name = "list", ty = "array", desc = "附件行（含 admin_name / entity_label / is_public）"},
    {name = "total", ty = "int", desc = "总条数"},
])]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<AttachQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:attach:list")?;
    let (page, size) = page_size(q.page, q.size);
    let scope = datascope::resolve(&auth, &state.db).await?;

    let mut qs = Attachment::query();
    if let Some(n) = q.name.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("name", n.trim()).map_err(ApiError::from)?;
    }
    match q.entity_type.as_deref().unwrap_or("").trim() {
        "none" => qs = qs.filter_eq("entity_type", "").map_err(ApiError::from)?,
        "" => {}
        other => qs = qs.filter_eq("entity_type", other).map_err(ApiError::from)?,
    }
    if let Some(raw) = q.entity_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        qs = qs
            .filter_eq("entity_id", crate::hid::dec(raw)?)
            .map_err(ApiError::from)?;
    }
    // 数据权限：按上传人收窄（与日志类同一套：`admin_id` 子查询形态）
    qs = datascope::apply_admin_scope(qs, &scope);

    let (rows, total) = qs
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;

    let (names, labels) = row_labels(&state, &rows).await?;
    let list: Vec<Value> = rows.iter().map(|a| row_json(a, &names, &labels)).collect();
    Ok(ok(json!({ "list": list, "total": total })))
}

/// 列表附带的两个「翻译表」：上传人昵称、关联对象的（标题, 是否已发布）。
/// 一次批量查完，别在行循环里逐条查库。
type Names = HashMap<i64, String>;
/// 关联对象 id → (标题, 是否公开可见)
type Labels = HashMap<i64, (String, bool)>;

async fn row_labels(state: &AppState, rows: &[Attachment]) -> Result<(Names, Labels), ApiError> {
    let mut admin_ids: Vec<i64> = rows.iter().map(|a| a.admin_id).collect();
    admin_ids.sort_unstable();
    admin_ids.dedup();
    let mut names = Names::new();
    if !admin_ids.is_empty() {
        let admins: Vec<Admin> = Admin::query()
            .filter_in("id", &crate::api::ids_to_values(&admin_ids))
            .map_err(ApiError::from)?
            .all(&state.db)
            .await
            .map_err(ApiError::from)?;
        names = admins.into_iter().map(|a| (a.id, a.nickname)).collect();
    }

    let mut notice_ids: Vec<i64> = rows
        .iter()
        .filter(|a| a.entity_type == "notice" && a.entity_id != 0)
        .map(|a| a.entity_id)
        .collect();
    notice_ids.sort_unstable();
    notice_ids.dedup();
    let mut labels = Labels::new();
    if !notice_ids.is_empty() {
        let notices: Vec<Notice> = Notice::query()
            .filter_in("id", &crate::api::ids_to_values(&notice_ids))
            .map_err(ApiError::from)?
            .all(&state.db)
            .await
            .map_err(ApiError::from)?;
        labels = notices
            .into_iter()
            .map(|n| (n.id, (n.title, n.status == 1)))
            .collect();
    }
    Ok((names, labels))
}

/// 一行的对外形状。**不经模型序列化**：`stored_path` 是内部地址，手工挑字段
/// 才不会哪天顺手把它带出去（判空逻辑也只有这一处）。
fn row_json(a: &Attachment, names: &Names, labels: &Labels) -> Value {
    let label = labels.get(&a.entity_id);
    json!({
        "id": hid::enc(a.id),
        "name": a.name,
        "ext": a.ext,
        "mime": a.mime,
        "size": a.size,
        "sha256": a.sha256,
        "admin_id": hid::enc(a.admin_id),
        "admin_name": names.get(&a.admin_id).cloned().unwrap_or_default(),
        "entity_type": a.entity_type,
        "entity_id": if a.entity_id == 0 { String::new() } else { hid::enc(a.entity_id) },
        "entity_label": label.map(|(t, _)| t.clone()).unwrap_or_default(),
        "is_public": label.map(|(_, public)| *public).unwrap_or(false),
        "sort": a.sort,
        "created_at": a.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

// ── 下载 ────────────────────────────────────────────────────

#[apidoc::title("下载附件")]
#[apidoc::desc("需要登录。绑定了**已发布**公告的附件对所有登录用户可下；其余按上传人的数据权限（超范围 403）。文件不存在 404")]
#[apidoc::url("/api/v1/attachments/{id}")]
#[apidoc::method("GET")]
#[apidoc::tag("附件")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "附件对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("401")]
#[apidoc::response_status("403")]
#[apidoc::response_status("404")]
pub async fn download(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Response, ApiError> {
    let row = load(&state, &id).await?;
    ensure_readable(&state, &auth, &row).await?;

    // 三段式路径由插件校验（`Runtime::resource`），库里被改脏也读不出目录外的文件
    let path = state
        .aether
        .resource(&row.stored_path)
        .map(|r| r.path)
        .filter(|p| p.is_file())
        .ok_or(ApiError::NotFound)?;
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ApiError::internal(format!("读附件 {} 失败: {e}", path.display())))?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&row.mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    // 非 ASCII 文件名会让 HeaderValue 构造失败 → 退化成裸 `attachment`，
    // 浏览器侧的文件名由前端 `a.download` 决定（与 csv 导出同一取舍）
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", row.name))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    Ok((headers, bytes).into_response())
}

/// 「能不能读这份附件」的唯一口径（下载用；列表另走数据权限）。
/// 1) 绑定了已发布公告 → 放行（公开）；
/// 2) 否则按上传人的数据权限。
async fn ensure_readable(state: &AppState, auth: &Auth, row: &Attachment) -> Result<(), ApiError> {
    if row.entity_type == "notice" && row.entity_id != 0 {
        let notice = Notice::query()
            .filter_eq("id", row.entity_id)
            .map_err(ApiError::from)?
            .one(&state.db)
            .await
            .map_err(ApiError::from)?;
        if notice.map(|n| n.status == 1).unwrap_or(false) {
            return Ok(());
        }
    }
    match Admin::query()
        .filter_eq("id", row.admin_id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
    {
        Some(uploader) => datascope::ensure_admin_in_scope(auth, &state.db, &uploader).await,
        // 上传人已删：孤儿附件谁都不给（超管兜底，否则管理员被删就再没人能清理）
        None => {
            let scope = datascope::resolve(auth, &state.db).await?;
            if scope.all {
                Ok(())
            } else {
                Err(ApiError::Forbidden("超出你的数据权限范围".into()))
            }
        }
    }
}

/// 短串 → 行（不存在一律 404）。
async fn load(state: &AppState, id: &str) -> Result<Attachment, ApiError> {
    let id = hid::dec(id)?;
    Attachment::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)
}

// ── 关联（多态绑定）────────────────────────────────────────

#[derive(Deserialize)]
pub struct BindBody {
    /// 关联类型：`notice` / 空串（解绑）
    #[serde(default)]
    pub entity_type: String,
    /// 关联对象 id（短串；空串 = 0）
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub sort: i32,
}

#[apidoc::title("关联附件")]
#[apidoc::desc("把附件挂到业务对象上（目前只支持 notice = 公告配图）；`entity_type` 传空串 = 解绑。绑定的公告**发布后**该附件对所有登录用户可下载。需要 `system:attach:edit` + 上传人在你的数据权限内")]
#[apidoc::url("/api/v1/attachments/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("附件")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "附件对外 id（hashids 短串）")]
#[apidoc::param(name = "entity_type", ty = "string", desc = "关联类型：notice / 空串（解绑）")]
#[apidoc::param(name = "entity_id", ty = "string", desc = "关联对象对外 id（短串）")]
#[apidoc::param(name = "sort", ty = "int", desc = "同组排序，0~999，默认 0")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("403")]
#[apidoc::response_status("404")]
pub async fn bind(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<BindBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:attach:edit")?;
    let mut row = load(&state, &id).await?;
    // 改关联会把附件变成「公开」（绑到已发布公告上），写路径必须卡数据权限
    ensure_writable(&state, &auth, &row).await?;

    let entity_type = body.entity_type.trim().to_string();
    if !entity_type.is_empty() && !ENTITY_TYPES.contains(&entity_type.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "不支持的关联类型：{entity_type}（可用：{}）",
            ENTITY_TYPES.join("、")
        )));
    }
    if !(0..=SORT_MAX).contains(&body.sort) {
        return Err(ApiError::BadRequest(format!("排序需在 0~{SORT_MAX} 之间")));
    }

    // 空串 = 0（解绑），其余必须是可解的短串（解不出由 `hid::dec` 给 400）
    let entity_id = if body.entity_id.trim().is_empty() {
        0
    } else {
        hid::dec(&body.entity_id)?
    };
    if entity_type == "notice" {
        if entity_id == 0 {
            return Err(ApiError::BadRequest("关联公告不能为空".into()));
        }
        // 关联的另一头必须真实存在，否则库里会留下查不出来的幽灵关联
        let exists = Notice::query()
            .filter_eq("id", entity_id)
            .map_err(ApiError::from)?
            .one(&state.db)
            .await
            .map_err(ApiError::from)?;
        if exists.is_none() {
            return Err(ApiError::NotFoundMsg("公告不存在".into()));
        }
    } else {
        // 解绑：两个字段一起清，不留半拉关联
        row.entity_id = 0;
    }
    row.entity_type = entity_type;
    if row.entity_type == "notice" {
        row.entity_id = entity_id;
    }
    row.sort = body.sort;
    row.updated_at = now();
    row.update(&state.db).await.map_err(ApiError::from)?;

    let (names, labels) = row_labels(&state, std::slice::from_ref(&row)).await?;
    Ok(ok(row_json(&row, &names, &labels)))
}

/// 写路径（改关联 / 删除）的数据权限：与下载的规则 2 同源，但**不看公开规则**
/// —— 能下到一张公开配图不等于能改它。
async fn ensure_writable(state: &AppState, auth: &Auth, row: &Attachment) -> Result<(), ApiError> {
    match Admin::query()
        .filter_eq("id", row.admin_id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
    {
        Some(uploader) => datascope::ensure_admin_in_scope(auth, &state.db, &uploader).await,
        None => {
            let scope = datascope::resolve(auth, &state.db).await?;
            if scope.all {
                Ok(())
            } else {
                Err(ApiError::Forbidden("超出你的数据权限范围".into()))
            }
        }
    }
}

// ── 删除 ────────────────────────────────────────────────────

#[apidoc::title("删除附件")]
#[apidoc::desc("需要 `system:attach:remove` + 上传人在你的数据权限内（绑定公开公告也不放宽）。**引用计数**：同一份内容被别的附件行引用时只删行，文件留到最后一份删掉时才清理")]
#[apidoc::url("/api/v1/attachments/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("附件")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "附件对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("403")]
#[apidoc::response_status("404")]
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:attach:remove")?;
    let row = load(&state, &id).await?;
    ensure_writable(&state, &auth, &row).await?;

    crate::api::delete_by_ids(&state, "attachment", &[row.id]).await?;

    // 引用计数：内容相同的附件共用同一个成品文件（同 md5 落同一个路径 + 秒传索引），
    // 还有别的行指着它就只删行。本行已删，剩下匹配的都是别人。
    let others: i64 = Attachment::query()
        .filter_eq("stored_path", row.stored_path.clone())
        .map_err(ApiError::from)?
        .count(&state.db)
        .await
        .map_err(ApiError::from)?;
    if others == 0 {
        // 交给插件删：文件 + 秒传索引一起清（索引留在那儿，下次重传同一份内容会
        // 「秒传命中」一个已经不存在的路径）。清索引单独再走一遍，理由同 avatar::finish
        if !state.aether.delete_resource(&row.stored_path) {
            tracing::debug!("删附件文件 {} 未成功（可能已被删或路径非法）", row.stored_path);
        }
        if !state.aether.delete_instant_path(&row.stored_path) {
            tracing::warn!("清附件的秒传索引 {} 未成功（重传同一份内容可能命中死链）", row.stored_path);
        }
    }
    Ok(ok(json!({ "deleted": 1 })))
}

// ── 表单 ────────────────────────────────────────────────────

/// multipart → 插件 `FormData`，顺带取出**原始文件名**（`name` 字段；插件不认它，
/// 落库与下载要用）。字段名与头像那套逐字一致（`resource_*`）。
async fn read_form(multipart: &mut Multipart) -> Result<(FormData, String), ApiError> {
    let mut form = FormData::new();
    let mut name = String::new();

    while let Some(field) = multipart.next_field().await.map_err(form_error)? {
        let Some(key) = field.name().map(str::to_string) else {
            continue;
        };
        if key == "resource_chunk" {
            let bytes = field.bytes().await.map_err(form_error)?;
            // 单片不该比整份上限还大：超了说明客户端没按服务端给的 chunk_size 切
            if bytes.len() as u64 > ATTACH_MAX_BYTES {
                return Err(too_large_err());
            }
            form.push_file(key, ChunkBody::Bytes(bytes.to_vec()));
            continue;
        }
        let text = field.text().await.map_err(form_error)?;
        if key == "name" {
            name = text;
            continue; // 插件不认这个字段，不往里塞
        }
        if key == "resource_name" && name.is_empty() {
            name = text.clone();
        }
        if key == "resource_size"
            && let Ok(n) = text.trim().parse::<u64>()
            && n > ATTACH_MAX_BYTES
        {
            return Err(too_large_err());
        }
        form.push_text(key, text);
    }

    // 分组名服务端钉死：它决定落盘目录，不能让客户端挑（插件按 `group` 查分组配置）
    form.push_text("group", ATTACH_GROUP);
    Ok((form, name))
}

fn form_error(e: axum::extract::multipart::MultipartError) -> ApiError {
    ApiError::BadRequest(format!("上传表单解析失败: {e}"))
}

/// 插件成功体原文 → `Value`（字段名原文透传，与头像一致）。
fn plugin_json(raw: &str) -> Result<Value, ApiError> {
    serde_json::from_str(raw)
        .map_err(|e| ApiError::internal(format!("解析上传响应失败: {e}（原文 {raw}）")))
}

/// 按 `[app] upload_dir` 装配时给附件组用的分组配置（分组本体在
/// `api::avatar::build_runtime` 里组装，这里只导出参数，免得两处各写一遍白名单）。
pub(crate) fn group() -> aetherupload::GroupConfig {
    aetherupload::GroupConfig {
        group_dir: ATTACH_GROUP.to_string(),
        resource_maxsize: ATTACH_MAX_BYTES,
        resource_extensions: ATTACH_EXTS.iter().map(|e| e.to_string()).collect(),
        event_before_upload_complete: false,
        event_upload_complete: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_name_and_whitelist_are_plugin_safe() {
        // 分组名含下划线会让 saved_path 三段式解码错位（插件直接拒）
        assert!(!ATTACH_GROUP.contains('_'));
        assert!(ATTACH_GROUP.len() <= 20);
        // 白名单里不能有可执行扩展名（黑名单那道只在白名单为空时才生效）
        for ext in ATTACH_EXTS {
            assert!(!matches!(ext, "php" | "exe" | "sh" | "js" | "jar" | "html" | "svg"), "{ext}");
        }
        // 内容反查用的扩展名（同族）也要在名单里，否则末片的内容校验过不去
        for ext in ["png", "jpg", "txt", "zip"] {
            assert!(ATTACH_EXTS.contains(&ext), "内容反查可能得出 {ext}");
        }
        // 旧版 office（OLE2）判成 octet-stream → 反查 bin，故意不收
        assert!(!ATTACH_EXTS.contains(&"bin"));
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sanitize_name_strips_paths_and_controls() {
        assert_eq!(sanitize_name("logo.png", "attach_x_y.png"), "logo.png");
        // 路径分隔符 / 控制字符 / 引号都换成空格（防「../」与引号破坏响应头）
        assert_eq!(sanitize_name("../../etc/passwd", "attach_x_y.png"), ".. .. etc passwd");
        assert_eq!(sanitize_name("a\"b\nc.png", "attach_x_y.png"), "a b c.png");
        // 空 / 全清掉的 → 退回落盘文件名（别给一个空名字）
        assert_eq!(sanitize_name("   ", "attach_202610_deadbeef.png"), "deadbeef.png");
        assert_eq!(sanitize_name("中文名.pdf", "attach_x_y.pdf"), "中文名.pdf");
        // 限长到列宽
        assert_eq!(sanitize_name(&"x".repeat(300), "attach_x_y.png").chars().count(), NAME_MAX);
    }
}
