// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;

// ── 字典项 ──────────────────────────────────────────────────

#[derive(Deserialize, Clone)]
pub struct DictItemQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub type_code: Option<String>,
    pub label: Option<String>,
    pub status: Option<i8>,
}

#[derive(Deserialize)]
pub struct DictItemCreate {
    pub type_code: String,
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub sort: i32,
    #[serde(default = "one")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
}

/// 与类型同理：`type_code` 是关联键，更新体里没有它（前端提交了也忽略）。
#[derive(Deserialize)]
pub struct DictItemUpdate {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub sort: i32,
    #[serde(default = "one")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
}

pub(crate) fn check_label(label: &str) -> Result<(), ApiError> {
    if label.trim().is_empty() {
        return Err(ApiError::BadRequest("字典标签不能为空".into()));
    }
    check_len("label", label.trim(), 64)
}

pub(crate) fn filtered_items(q: &DictItemQuery) -> Result<bee_orm::QuerySet<DictItem>, ApiError> {
    let mut qs = DictItem::query();
    if let Some(c) = q.type_code.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_eq("type_code", c.trim()).map_err(ApiError::from)?;
    }
    if let Some(l) = q.label.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("label", l.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    Ok(qs)
}

#[apidoc::title("字典项列表")]
#[apidoc::desc("按类型 code + 标签模糊筛选 + 分页")]
#[apidoc::url("/api/v1/dict-items")]
#[apidoc::method("GET")]
#[apidoc::tag("字典")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100")]
#[apidoc::query(name = "type_code", ty = "string", desc = "所属字典标识 code")]
#[apidoc::query(name = "label", ty = "string", desc = "标签模糊匹配")]
#[apidoc::query(name = "status", ty = "int", desc = "状态：1 启用 / 0 停用")]
#[apidoc::response_status("200")]
pub async fn item_list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<DictItemQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:list")?;
    let (page, size) = page_size(q.page, q.size);
    let (rows, total) = filtered_items(&q)?
        // 与下拉同序：管理页看到的顺序就是前端下拉的顺序
        .order_by("sort ASC, id ASC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

#[apidoc::title("新建字典项")]
#[apidoc::desc("同一类型下 value 唯一；label 超长回 400")]
#[apidoc::url("/api/v1/dict-items")]
#[apidoc::method("POST")]
#[apidoc::tag("字典")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "type_code", ty = "string", required, desc = "所属字典标识 code（必须已存在）")]
#[apidoc::param(name = "label", ty = "string", required, desc = "展示文案")]
#[apidoc::param(name = "value", ty = "string", required, desc = "实际值（同类型下唯一）")]
#[apidoc::param(name = "sort", ty = "int", desc = "排序，默认 0")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
pub async fn item_create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<DictItemCreate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:add")?;
    check_label(&body.label)?;
    check_value(&body.value)?;
    check_len("remark", &body.remark, 255)?;
    let type_code = body.type_code.trim();
    if !type_exists(&state, type_code).await? {
        return Err(ApiError::BadRequest("字典类型不存在".into()));
    }

    let it = DictItem {
        id: state.next_id()?,
        type_code: type_code.to_string(),
        label: body.label.trim().to_string(),
        value: body.value.trim().to_string(),
        sort: body.sort,
        status: body.status,
        remark: body.remark,
        created_at: now(),
        updated_at: now(),
    };
    // id 已在上面发号，insert 就够（create 的读回是给自增主键用的）
    it.insert(&state.db).await.map_err(value_taken)?;
    Ok(ok(json!({ "id": hid::enc(it.id) })))
}

#[apidoc::title("编辑字典项")]
#[apidoc::desc("更新体里没有 type_code：它是关联键（前端提交了也忽略）")]
#[apidoc::url("/api/v1/dict-items/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("字典")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "字典项对外 id（hashids 短串）")]
#[apidoc::param(name = "label", ty = "string", required, desc = "展示文案")]
#[apidoc::param(name = "value", ty = "string", required, desc = "实际值（同类型下唯一）")]
#[apidoc::param(name = "sort", ty = "int", desc = "排序")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
pub async fn item_update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<DictItemUpdate>,
) -> Result<(AuditDetail, Json<Value>), ApiError> {
    auth.require("system:dict:edit")?;
    let id = hid::dec(&id)?;
    check_label(&body.label)?;
    check_value(&body.value)?;
    check_len("remark", &body.remark, 255)?;

    let mut it = DictItem::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    let old = it.clone(); // 变更摘要的旧值（下面就地改 it）

    it.label = body.label.trim().to_string();
    it.value = body.value.trim().to_string();
    it.sort = body.sort;
    it.status = body.status;
    it.remark = body.remark;
    it.updated_at = now();
    // 变更摘要（项 8）：type_code 是关联键、更新体里就没有，天然不会出现在摘要里
    let changes = Diff::default()
        .field("label", old.label, it.label.clone())
        .field("value", old.value, it.value.clone())
        .field("sort", old.sort, it.sort)
        .field("status", old.status, it.status)
        .field("remark", old.remark, it.remark.clone())
        .done();
    it.update(&state.db).await.map_err(value_taken)?;
    Ok((changes, ok(Value::Null)))
}

#[apidoc::title("删除字典项")]
#[apidoc::desc("不存在回 404")]
#[apidoc::url("/api/v1/dict-items/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("字典")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "字典项对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
pub async fn item_remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:remove")?;
    let id = hid::dec(&id)?;
    let n = DictItem::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .delete(&state.db)
        .await
        .map_err(ApiError::from)?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(ok(Value::Null))
}

/// 取一批导出数据（keyset：`id < last`，首页不带该条件）。
/// `QuerySet` 不可克隆，所以每批用筛选条件重建（与列表共用 `filtered_items()`）。
pub(crate) async fn export_batch(
    state: &AppState,
    q: &DictItemQuery,
    last: Option<i64>,
) -> Result<Vec<(i64, String)>, ApiError> {
    let mut qs = filtered_items(q)?.order_by("id DESC").limit(csv::BATCH);
    if let Some(last) = last {
        qs = qs.filter_lt("id", last).map_err(ApiError::from)?;
    }
    let rows = qs.all(&state.db).await.map_err(ApiError::from)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.id,
                csv::row(&[
                    hid::enc(r.id),
                    r.type_code,
                    r.label,
                    r.value,
                    r.sort.to_string(),
                    if r.status == 1 { "启用".into() } else { "停用".into() },
                    r.remark,
                    r.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                ]),
            )
        })
        .collect())
}

/// 导出当前筛选结果（不含分页）：筛选条件与列表共用 `filtered_items()`，导出范围不会漂移。
#[apidoc::title("导出字典项 CSV")]
#[apidoc::desc("导出当前筛选结果（不含分页，流式），筛选条件与列表共用")]
#[apidoc::url("/api/v1/dict-items/export")]
#[apidoc::method("GET")]
#[apidoc::tag("字典")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "type_code", ty = "string", desc = "所属字典标识 code")]
#[apidoc::query(name = "label", ty = "string", desc = "标签模糊匹配")]
#[apidoc::query(name = "status", ty = "int", desc = "状态：1 启用 / 0 停用")]
#[apidoc::response_status("200")]
#[apidoc::not_debug]
pub async fn item_export(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<DictItemQuery>,
) -> Result<Response, ApiError> {
    auth.require("system:dict:list")?;
    csv::streamed(
        format!("dict-items-{}.csv", csv::today()),
        header_cells(),
        move |last| {
            let state = state.clone();
            let q = q.clone();
            async move { export_batch(&state, &q, last).await }
        },
    )
    .await
}

pub(crate) fn header_cells() -> Vec<String> {
    ["ID", "类型编码", "标签", "值", "排序", "状态", "备注", "创建时间"]
        .map(String::from)
        .to_vec()
}
