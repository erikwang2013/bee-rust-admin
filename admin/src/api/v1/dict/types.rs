// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;

pub(crate) fn one() -> i8 {
    1
}

/// 字典编码重复 → 本接口的专属文案（`dict.code_taken`）。
/// 唯一键冲突的识别在 `ApiError::from` 里按 MySQL 报错文本做（上游 `OrmError`
/// 没有独立变体），冲突是它唯一映射成 `BadRequest` 的情况，其余是 `NotFound` / `Internal`。
pub(crate) fn code_taken(e: OrmError) -> ApiError {
    match ApiError::from(e) {
        ApiError::BadRequest(_) => ApiError::BadRequest("字典编码已存在".into()),
        other => other,
    }
}

/// 同类型下字典值重复 → `dict.value_taken`（同上）。
pub(crate) fn value_taken(e: OrmError) -> ApiError {
    match ApiError::from(e) {
        ApiError::BadRequest(_) => ApiError::BadRequest("该类型下字典值已存在".into()),
        other => other,
    }
}

/// 字典编码：小写字母/数字/下划线，2~64 位。它既是字典项的关联键，
/// 又是接口路径段（`/dicts/{code}/items`），限死字符集就省掉了两头转义。
pub(crate) fn valid_code(code: &str) -> bool {
    let n = code.chars().count();
    (2..=64).contains(&n)
        && code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub(crate) fn check_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest("字典名称不能为空".into()));
    }
    check_len("name", name.trim(), 64)
}

pub(crate) fn check_value(value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::BadRequest("字典值不能为空".into()));
    }
    check_len("value", value.trim(), 64)
}

// ── 字典类型 ────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DictTypeQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub name: Option<String>,
    pub status: Option<i8>,
}

#[derive(Deserialize)]
pub struct DictTypeCreate {
    pub name: String,
    pub code: String,
    #[serde(default = "one")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
}

/// code 是关联键，改它等于把已有字典项全变成孤儿 —— 更新体里干脆没有这个字段。
#[derive(Deserialize)]
pub struct DictTypeUpdate {
    pub name: String,
    #[serde(default = "one")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
}

pub(crate) fn filtered_types(q: &DictTypeQuery) -> Result<bee_orm::QuerySet<DictType>, ApiError> {
    let mut qs = DictType::query();
    if let Some(n) = q.name.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("name", n.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    Ok(qs)
}

#[apidoc::title("字典类型列表")]
#[apidoc::desc("字典名模糊 + 状态筛选 + 分页")]
#[apidoc::url("/api/v1/dicts")]
#[apidoc::method("GET")]
#[apidoc::tag("字典")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100")]
#[apidoc::query(name = "name", ty = "string", desc = "字典名模糊匹配")]
#[apidoc::query(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用")]
#[apidoc::response_status("200")]
pub async fn type_list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<DictTypeQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:list")?;
    let (page, size) = page_size(q.page, q.size);
    let (rows, total) = filtered_types(&q)?
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

#[apidoc::title("新建字典类型")]
#[apidoc::desc("code 唯一且只允许小写字母/数字/下划线（唯一键冲突回 400）")]
#[apidoc::url("/api/v1/dicts")]
#[apidoc::method("POST")]
#[apidoc::tag("字典")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "code", ty = "string", required, desc = "字典标识（唯一，关联字典项）")]
#[apidoc::param(name = "name", ty = "string", required, desc = "字典名")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
pub async fn type_create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<DictTypeCreate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:add")?;
    check_name(&body.name)?;
    check_len("remark", &body.remark, 255)?;
    let code = body.code.trim();
    if !valid_code(code) {
        return Err(ApiError::BadRequest(
            "字典编码只能是小写字母、数字、下划线，长度 2~64".into(),
        ));
    }

    let t = DictType {
        id: state.next_id()?,
        name: body.name.trim().to_string(),
        code: code.to_string(),
        status: body.status,
        remark: body.remark,
        created_at: now(),
        updated_at: now(),
    };
    // id 已在上面发号，insert 就够（create 的读回是给自增主键用的）
    t.insert(&state.db).await.map_err(code_taken)?;
    Ok(ok(json!({ "id": hid::enc(t.id) })))
}

#[apidoc::title("编辑字典类型")]
#[apidoc::desc("更新体里没有 code：它是关联键，改了会把已有字典项变成孤儿")]
#[apidoc::url("/api/v1/dicts/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("字典")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "字典类型对外 id（hashids 短串）")]
#[apidoc::param(name = "name", ty = "string", required, desc = "字典名")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
pub async fn type_update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<DictTypeUpdate>,
) -> Result<(AuditDetail, Json<Value>), ApiError> {
    auth.require("system:dict:edit")?;
    let id = hid::dec(&id)?;
    check_name(&body.name)?;
    check_len("remark", &body.remark, 255)?;

    let mut t = DictType::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    let old = t.clone(); // 变更摘要的旧值（下面就地改 t）

    t.name = body.name.trim().to_string();
    t.status = body.status;
    t.remark = body.remark;
    t.updated_at = now();
    // 变更摘要（项 8）：code 是关联键、更新体里就没有，天然不会出现在摘要里
    let changes = Diff::default()
        .field("name", old.name, t.name.clone())
        .field("status", old.status, t.status)
        .field("remark", old.remark, t.remark.clone())
        .done();
    t.update(&state.db).await.map_err(code_taken)?;
    Ok((changes, ok(Value::Null)))
}

/// 删类型连带删它的字典项，同一事务：先按 `type_code` 一条 DELETE 清项，再删类型，
/// 中途失败回滚，不留「类型没了项还在」的孤儿行。
#[apidoc::title("删除字典类型")]
#[apidoc::desc("连带删它的字典项，**同一事务**（不留「类型没了项还在」的孤儿行）")]
#[apidoc::url("/api/v1/dicts/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("字典")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "字典类型对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
pub async fn type_remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:remove")?;
    let id = hid::dec(&id)?;
    let t = DictType::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    // Db 层没有事务：事务开在 Pool::get() 拿到的连接上（语句走连接，不能走池）
    let mut conn = state.db.get().await.map_err(ApiError::from)?;
    conn.begin().await.map_err(ApiError::from)?;
    let result = async {
        let items_deleted = conn
            .execute("DELETE FROM dict_item WHERE type_code = ?", &[DbValue::from(t.code.as_str())])
            .await?;
        conn.execute("DELETE FROM dict_type WHERE id = ?", &[DbValue::from(t.id)]).await?;
        Ok::<u64, OrmError>(items_deleted)
    }
    .await;
    let items_deleted = match result {
        Ok(n) => {
            conn.commit().await.map_err(ApiError::from)?;
            n
        }
        Err(e) => {
            let _ = conn.rollback().await;
            return Err(e.into());
        }
    };

    Ok(ok(json!({ "items_deleted": items_deleted })))
}

/// 类型必须存在：不建外键，至少别让手滑的 `type_code` 造出孤儿项（下拉里永远看不到）。
pub(crate) async fn type_exists(state: &AppState, code: &str) -> Result<bool, ApiError> {
    Ok(DictType::query()
        .filter_eq("code", code)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .is_some())
}

/// 下拉数据源：只认登录，不要求 `system:dict:list`（任何登录用户都要能取字典）。
/// 只回启用项，按 `sort, id` 排序；字段只留前端要的 `label` / `value`。
///
/// 类型不存在回 404（拼错 code 的人该看到「类型不存在」而不是一个空下拉，
/// 分不清「没配数据」和「写错了」）；类型在但没启用项回 200 + `[]`——那是合法状态。
#[apidoc::title("按 code 取字典项下拉数据")]
#[apidoc::desc("登录即可（不挂权限码）。类型不存在回 404（拼错 code 该看到「类型不存在」，而不是一个空下拉）；类型在但没启用项回 200 + 空数组")]
#[apidoc::url("/api/v1/dicts/{code}/items")]
#[apidoc::method("GET")]
#[apidoc::tag("字典")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "code", ty = "string", required, desc = "字典标识 code")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
#[apidoc::returned(name = "data", ty = "array", desc = "启用中的字典项 [{ value, label }, …]")]
pub async fn type_items(
    State(state): State<AppState>,
    _auth: Auth,
    AppPath(code): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    if !type_exists(&state, &code).await? {
        return Err(ApiError::NotFoundMsg("字典类型不存在".into()));
    }

    let rows = DictItem::query()
        .filter_eq("type_code", &code)
        .map_err(ApiError::from)?
        .filter_eq("status", 1)
        .map_err(ApiError::from)?
        .order_by("sort ASC, id ASC")
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|i| json!({ "label": i.label, "value": i.value }))
        .collect();
    Ok(ok(out))
}
