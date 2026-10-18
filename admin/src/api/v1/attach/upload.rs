// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;

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
pub(crate) const NAME_MAX: usize = 255;

/// 允许关联的对象类型。**白名单**：多态关联的另一头要能查得到、判得出「公不公开」，
/// 放开任意字符串等于允许往库里写查不出来的幽灵关联。
pub(crate) const ENTITY_TYPES: [&str; 1] = ["notice"];

/// 关联排序的取值范围（同组配图顺序，前端按它排）。
pub(crate) const SORT_MAX: i32 = 999;

// ── 表单 ────────────────────────────────────────────────────

/// multipart → 插件 `FormData`，顺带取出**原始文件名**（`name` 字段；插件不认它，
/// 落库与下载要用）。字段名与头像那套逐字一致（`resource_*`）。
pub(crate) async fn read_form(multipart: &mut Multipart) -> Result<(FormData, String), ApiError> {
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

pub(crate) fn form_error(e: axum::extract::multipart::MultipartError) -> ApiError {
    ApiError::BadRequest(format!("上传表单解析失败: {e}"))
}

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
pub(crate) async fn finish(
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
pub(crate) fn sanitize_name(name: &str, saved_path: &str) -> String {
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
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// 单文件上限（`attach.too_large`）：自己拦一道，给稳定码与准确文案；插件那道
/// `payload_too_large`（413）当兜底。
pub(crate) fn too_large_err() -> ApiError {
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
pub(crate) async fn check_quota(state: &AppState, auth: &Auth, form: &FormData) -> Result<(), ApiError> {
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
pub(crate) async fn check_quota_bytes(state: &AppState, admin_id: i64, incoming: u64) -> Result<(), ApiError> {
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
pub(crate) async fn sum_size(state: &AppState, admin_id: i64) -> Result<u64, ApiError> {
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
