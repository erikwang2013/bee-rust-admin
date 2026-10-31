// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;
use axum::body::{Body, Bytes};
use tokio::io::AsyncReadExt;
use tokio_stream::wrappers::ReceiverStream;

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
#[apidoc::desc(
    "按文件名模糊 / 关联类型筛选 + 分页；受数据权限约束（只看得到自己范围内的上传人）。需要 `system:attach:list`"
)]
#[apidoc::url("/api/v1/attachments")]
#[apidoc::method("GET")]
#[apidoc::tag("附件")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100")]
#[apidoc::query(name = "name", ty = "string", desc = "原始文件名模糊匹配")]
#[apidoc::query(name = "entity_type", ty = "string", desc = "关联类型：notice / none（未绑定）")]
#[apidoc::query(
    name = "entity_id",
    ty = "string",
    desc = "关联对象短串（配合 entity_type 精确定位）"
)]
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
        qs = qs.filter_eq("entity_id", crate::hid::dec(raw)?).map_err(ApiError::from)?;
    }
    // 数据权限：按上传人收窄（与日志类同一套：`admin_id` 子查询形态）
    qs = datascope::apply_admin_scope(qs, &scope);

    let (rows, total) =
        qs.order_by("id DESC").fetch_page(&state.db, page, size).await.map_err(ApiError::from)?;

    let (names, labels) = row_labels(&state, &rows).await?;
    let list: Vec<Value> = rows.iter().map(|a| row_json(a, &names, &labels)).collect();
    Ok(ok(json!({ "list": list, "total": total })))
}

/// 列表附带的两个「翻译表」：上传人昵称、关联对象的（标题, 是否已发布）。
/// 一次批量查完，别在行循环里逐条查库。
pub(crate) type Names = HashMap<i64, String>;
/// 关联对象 id → (标题, 是否公开可见)
pub(crate) type Labels = HashMap<i64, (String, bool)>;

pub(crate) async fn row_labels(
    state: &AppState,
    rows: &[Attachment],
) -> Result<(Names, Labels), ApiError> {
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
        labels = notices.into_iter().map(|n| (n.id, (n.title, n.status == 1))).collect();
    }
    Ok((names, labels))
}

/// 一行的对外形状。**不经模型序列化**：`stored_path` 是内部地址，手工挑字段
/// 才不会哪天顺手把它带出去（判空逻辑也只有这一处）。
pub(crate) fn row_json(a: &Attachment, names: &Names, labels: &Labels) -> Value {
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
#[apidoc::desc(
    "需要登录。绑定了**已发布**公告的附件对所有登录用户可下；其余按上传人的数据权限（超范围 403）。文件不存在 404"
)]
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
    // 流式回文件（与 csv::stream_file 同款）：附件最大 10MB，整读进内存后并发下载会成倍放大
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| ApiError::internal(format!("读附件 {} 失败: {e}", path.display())))?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(4);
    tokio::spawn(async move {
        let mut file = file;
        // 64 KiB 一块：太小则 syscall 多，太大则白占内存
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match file.read(&mut buf).await {
                Ok(0) => return,
                // 发送失败 = 客户端断开：收手
                Ok(n) => {
                    if tx.send(Ok(Bytes::copy_from_slice(&buf[..n]))).await.is_err() {
                        return;
                    }
                }
                // 状态码早已发出、改不了：中断 body 让客户端看到「传输失败」
                Err(e) => {
                    tracing::error!("附件下载中断: {e}");
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            }
        }
    });

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&row.mime)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    // 非 ASCII 文件名会让 HeaderValue 构造失败 → 退化成裸 `attachment`，
    // 浏览器侧的文件名由前端 `a.download` 决定（与 csv 导出同一取舍）
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", row.name))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    Ok((headers, Body::from_stream(ReceiverStream::new(rx))).into_response())
}

/// 「能不能读这份附件」的唯一口径（下载用；列表另走数据权限）。
/// 1) 绑定了已发布公告 → 放行（公开）；
/// 2) 否则按上传人的数据权限。
pub(crate) async fn ensure_readable(
    state: &AppState,
    auth: &Auth,
    row: &Attachment,
) -> Result<(), ApiError> {
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
pub(crate) async fn load(state: &AppState, id: &str) -> Result<Attachment, ApiError> {
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
#[apidoc::desc(
    "把附件挂到业务对象上（目前只支持 notice = 公告配图）；`entity_type` 传空串 = 解绑。绑定的公告**发布后**该附件对所有登录用户可下载。需要 `system:attach:edit` + 上传人在你的数据权限内"
)]
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
    let entity_id = if body.entity_id.trim().is_empty() { 0 } else { hid::dec(&body.entity_id)? };
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
pub(crate) async fn ensure_writable(
    state: &AppState,
    auth: &Auth,
    row: &Attachment,
) -> Result<(), ApiError> {
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
#[apidoc::desc(
    "需要 `system:attach:remove` + 上传人在你的数据权限内（绑定公开公告也不放宽）。**只删记录行，文件不立即删**（S2 竞态硬化）：成品文件统一由内置任务「附件孤儿清理」在保护期（5 分钟）后收走，避免与「上传末片落盘 → 落库」的窗口打架"
)]
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

    // S2（批次 E）：这里**只删行，不删文件**。原先的「数一遍还有没有别的行引用、没人引用
    // 就立刻删文件」有个窗口：删行与删文件之间，同一份内容正在被别的会话重传/引用
    // （内容寻址：同 md5 落同一路径），或者插件刚写完文件还没 INSERT —— 立刻删就是把
    // 正要用的文件删掉。文件改由「附件孤儿清理」统一收（保护期后、且确认无行引用）。
    // 代价：删了附件不会马上还磁盘，最迟多留到下一轮清理（默认一天一次）；
    // 秒传索引同理留到那时一起清 —— 这段窗口里文件还在，重传正常命中，不会成死链。
    Ok(ok(json!({ "deleted": 1 })))
}

/// 插件成功体原文 → `Value`（字段名原文透传，与头像一致）。
pub(crate) fn plugin_json(raw: &str) -> Result<Value, ApiError> {
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
