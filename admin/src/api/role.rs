// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{PagingExt, check_len, dedup_ids, page_size};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::hid;
use crate::models::Role;
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use bee_orm::{Model, OrmError};
use serde::Deserialize;
use serde_json::{Value, json};
use crate::relations::RelationsExt;

#[derive(Deserialize)]
pub struct RoleListQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub name: Option<String>,
    pub status: Option<i8>,
}

#[derive(Deserialize)]
pub struct RoleBody {
    pub name: String,
    pub code: String,
    #[serde(default)]
    pub sort: i32,
    pub data_scope: i8,
    #[serde(default = "status_default")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
}

#[derive(Deserialize)]
pub struct MenuIdsBody {
    #[serde(deserialize_with = "crate::hid::de_vec_id")]
    pub menu_ids: Vec<i64>,
}

#[derive(Deserialize)]
pub struct DeptIdsBody {
    #[serde(deserialize_with = "crate::hid::de_vec_id")]
    pub dept_ids: Vec<i64>,
}

fn status_default() -> i8 {
    1
}

fn validate_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest("角色名称不能为空".into()));
    }
    Ok(())
}

fn validate_code(code: &str) -> Result<(), ApiError> {
    if code.chars().count() < 3 || code.chars().count() > 64 {
        return Err(ApiError::BadRequest("角色标识长度需 3-64".into()));
    }
    if !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':') {
        return Err(ApiError::BadRequest("角色标识只能包含字母、数字、下划线、冒号".into()));
    }
    Ok(())
}

fn validate_scope(scope: i8) -> Result<(), ApiError> {
    if !(1..=5).contains(&scope) {
        return Err(ApiError::BadRequest("数据范围取值需在 1-5 之间".into()));
    }
    Ok(())
}

/// 补 `grantable`（B5）：当前操作者能不能把这个角色授出去。
/// 只是显示提示 —— 前端角色选择框据此灰掉选项，保存时仍由
/// `ensure_roles_grantable` 硬校验（两处共用同一条规则）。
async fn with_grantable(
    state: &AppState,
    auth: &Auth,
    roles: Vec<Role>,
) -> Result<Vec<Value>, ApiError> {
    let grants = datascope::roles_grantable(auth, &state.db, &roles).await?;
    roles
        .into_iter()
        .zip(grants)
        .map(|(r, g)| {
            let mut v = serde_json::to_value(&r)
                .map_err(|e| ApiError::internal(format!("序列化失败: {e}")))?;
            v["grantable"] = json!(g);
            Ok(v)
        })
        .collect()
}

/// 长度上限与模型 `#[bee(len)]`、设计文档 §5.1 列宽一致（name/code 落库前会 trim）。
fn validate_body(b: &RoleBody) -> Result<(), ApiError> {
    validate_name(&b.name)?;
    validate_code(&b.code)?;
    validate_scope(b.data_scope)?;
    check_len("name", b.name.trim(), 64)?;
    check_len("code", b.code.trim(), 64)?;
    check_len("remark", &b.remark, 255)
}

#[apidoc::title("角色列表")]
#[apidoc::desc("名称模糊 + 状态筛选 + 分页；每条带 grantable（当前操作者能否把这个角色授出去）")]
#[apidoc::url("/api/v1/roles")]
#[apidoc::method("GET")]
#[apidoc::tag("角色")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1", mock = "1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100", mock = "10")]
#[apidoc::query(name = "name", ty = "string", desc = "角色名模糊匹配")]
#[apidoc::query(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "分页结果", children = [
            {name = "list", ty = "array", required, desc = "角色列表", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "name", ty = "string", required, desc = "角色名"},
            {name = "code", ty = "string", required, desc = "角色标识（权限码前缀）"},
            {name = "sort", ty = "int", desc = "排序（升序）"},
            {name = "data_scope", ty = "int", desc = "数据范围：1 全部 / 2 自定义 / 3 本部门 / 4 本部门及以下 / 5 仅本人"},
            {name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用"},
            {name = "grantable", ty = "bool", desc = "当前操作者能否把这个角色授出去（超管范围之外为 false）"},
            {name = "remark", ty = "string", desc = "备注"},
            {name = "created_at", ty = "string", desc = "创建时间"},
            {name = "updated_at", ty = "string", desc = "更新时间"},
        ]},
            {name = "total", ty = "int", required, desc = "筛选后的总条数"},
        ])]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<RoleListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:role:list")?;
    let (page, size) = page_size(q.page, q.size);

    let mut qs = Role::query();
    if let Some(n) = q.name.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("name", n.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }

    let (rows, total) = qs
        .order_by("sort ASC, id ASC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    let list = with_grantable(&state, &auth, rows).await?;
    Ok(ok(json!({ "list": list, "total": total })))
}

#[apidoc::title("角色详情")]
#[apidoc::desc("编辑弹窗回显，同样带 grantable")]
#[apidoc::url("/api/v1/roles/{id}")]
#[apidoc::method("GET")]
#[apidoc::tag("角色")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "角色对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
#[apidoc::returned(name = "data", ty = "object", desc = "角色对象", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "name", ty = "string", required, desc = "角色名"},
            {name = "code", ty = "string", required, desc = "角色标识（权限码前缀）"},
            {name = "sort", ty = "int", desc = "排序（升序）"},
            {name = "data_scope", ty = "int", desc = "数据范围：1 全部 / 2 自定义 / 3 本部门 / 4 本部门及以下 / 5 仅本人"},
            {name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用"},
            {name = "grantable", ty = "bool", desc = "当前操作者能否把这个角色授出去（超管范围之外为 false）"},
            {name = "remark", ty = "string", desc = "备注"},
            {name = "created_at", ty = "string", desc = "创建时间"},
            {name = "updated_at", ty = "string", desc = "更新时间"},
        ])]
pub async fn detail(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:role:list")?;
    let id = hid::dec(&id)?;
    let r = Role::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    // 详情同样带 grantable（前端编辑弹窗里也要判）
    let v = with_grantable(&state, &auth, vec![r]).await?.pop().unwrap_or(Value::Null);
    Ok(ok(v))
}

/// 角色标识重复 → 该接口的专属文案（`role.code_taken`）。
/// 唯一键冲突的识别在 `ApiError::from` 里按 MySQL 报错文本做（上游 `OrmError`
/// 没有独立变体），冲突是它唯一映射成 `BadRequest` 的情况，其余是 `NotFound` / `Internal`。
fn code_taken(e: OrmError) -> ApiError {
    match ApiError::from(e) {
        ApiError::BadRequest(_) => ApiError::BadRequest("角色标识已存在".into()),
        other => other,
    }
}

#[apidoc::title("新建角色")]
#[apidoc::desc("角色标识唯一；标识重复回 400（专属文案 role.code_taken）")]
#[apidoc::url("/api/v1/roles")]
#[apidoc::method("POST")]
#[apidoc::tag("角色")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "name", ty = "string", required, desc = "角色名，最长 64 字符")]
#[apidoc::param(name = "code", ty = "string", required, desc = "角色标识（唯一），最长 64 字符")]
#[apidoc::param(name = "sort", ty = "int", desc = "排序，默认 0")]
#[apidoc::param(name = "data_scope", ty = "int", required, desc = "数据范围：1 全部 / 2 自定义 / 3 本部门 / 4 本部门及以下 / 5 仅本人")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注，最长 255 字符")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "object", desc = "新建结果", children = [
            {name = "id", ty = "string", required, desc = "新角色对外 id（hashids 短串）"},
        ])]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<RoleBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:role:add")?;
    validate_body(&body)?;

    let r = Role {
        id: state.next_id()?,
        name: body.name.trim().to_string(),
        code: body.code.trim().to_string(),
        sort: body.sort,
        data_scope: body.data_scope,
        status: body.status,
        remark: body.remark,
        created_at: now(),
        updated_at: now(),
    };
    // id 已在上面发号，insert 就够（create 的读回是给自增主键用的）
    r.insert(&state.db).await.map_err(code_taken)?;
    Ok(ok(json!({ "id": hid::enc(r.id) })))
}

#[apidoc::title("编辑角色")]
#[apidoc::desc("字段与新建一致（整批提交）；角色名与标识重复回 400")]
#[apidoc::url("/api/v1/roles/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("角色")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "角色对外 id（hashids 短串）")]
#[apidoc::param(name = "name", ty = "string", required, desc = "角色名，最长 64 字符")]
#[apidoc::param(name = "code", ty = "string", required, desc = "角色标识（唯一），最长 64 字符")]
#[apidoc::param(name = "sort", ty = "int", desc = "排序，默认 0")]
#[apidoc::param(name = "data_scope", ty = "int", required, desc = "数据范围：1 全部 / 2 自定义 / 3 本部门 / 4 本部门及以下 / 5 仅本人")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注，最长 255 字符")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<RoleBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:role:edit")?;
    let id = hid::dec(&id)?;
    validate_body(&body)?;

    let mut r = Role::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    r.name = body.name.trim().to_string();
    r.code = body.code.trim().to_string();
    r.sort = body.sort;
    r.data_scope = body.data_scope;
    r.status = body.status;
    r.remark = body.remark;
    r.updated_at = now();
    r.update(&state.db).await.map_err(code_taken)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("删除角色")]
#[apidoc::desc("已被管理员使用时拒绝（400）；连带删角色-菜单、角色-部门关联")]
#[apidoc::url("/api/v1/roles/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("角色")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "角色对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:role:remove")?;
    let id = hid::dec(&id)?;
    let used = state.db.count_refs("admin_role", "role_id", id).await.map_err(ApiError::from)?;
    if used > 0 {
        return Err(ApiError::BadRequest("该角色已被管理员使用，不能删除".into()));
    }
    state.db.del_relations("role_menu", "role_id", id).await.map_err(ApiError::from)?;
    state.db.del_relations("role_dept", "role_id", id).await.map_err(ApiError::from)?;
    Role::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .delete(&state.db)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("角色的菜单 id 列表")]
#[apidoc::desc("前端「授权」弹窗的回显；data 直接是短串数组（不是对象）")]
#[apidoc::url("/api/v1/roles/{id}/menus")]
#[apidoc::method("GET")]
#[apidoc::tag("角色")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "角色对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "array", desc = "菜单短串数组")]
pub async fn get_menus(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:role:list")?;
    let id = hid::dec(&id)?;
    let ids = state
        .db
        .get_relations("role_menu", ("role_id", id), "menu_id")
        .await
        .map_err(ApiError::from)?;
    Ok(ok(hid::enc_vec(&ids))) // 前端契约：data 直接是 id 短串数组
}

#[apidoc::title("设置角色的菜单")]
#[apidoc::desc("整批覆盖；服务端把祖先节点补全后再落库（antd 半选的父节点不会提交）")]
#[apidoc::url("/api/v1/roles/{id}/menus")]
#[apidoc::method("PUT")]
#[apidoc::tag("角色")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "角色对外 id（hashids 短串）")]
#[apidoc::param(name = "menu_ids", ty = "array", required, desc = "菜单短串数组（整批覆盖，重复项自动去重）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn set_menus(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<MenuIdsBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:role:edit")?;
    let id = hid::dec(&id)?;
    let exists = Role::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?;
    if exists.is_none() {
        return Err(ApiError::NotFound);
    }
    state
        .db
        .set_relations("role_menu", ("role_id", id), "menu_id", &dedup_ids(body.menu_ids))
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("角色的部门 id 列表")]
#[apidoc::desc("数据范围 = 自定义时，前端回显的角色部门；data 直接是短串数组")]
#[apidoc::url("/api/v1/roles/{id}/depts")]
#[apidoc::method("GET")]
#[apidoc::tag("角色")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "角色对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "array", desc = "部门短串数组")]
pub async fn get_depts(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:role:list")?;
    let id = hid::dec(&id)?;
    let ids = state
        .db
        .get_relations("role_dept", ("role_id", id), "dept_id")
        .await
        .map_err(ApiError::from)?;
    Ok(ok(hid::enc_vec(&ids))) // 前端契约：data 直接是 id 短串数组
}

#[apidoc::title("设置角色的部门")]
#[apidoc::desc("整批覆盖；只在数据范围 = 自定义时有意义")]
#[apidoc::url("/api/v1/roles/{id}/depts")]
#[apidoc::method("PUT")]
#[apidoc::tag("角色")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "角色对外 id（hashids 短串）")]
#[apidoc::param(name = "dept_ids", ty = "array", required, desc = "部门短串数组（整批覆盖，重复项自动去重）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn set_depts(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<DeptIdsBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:role:edit")?;
    let id = hid::dec(&id)?;
    let exists = Role::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?;
    if exists.is_none() {
        return Err(ApiError::NotFound);
    }
    state
        .db
        .set_relations("role_dept", ("role_id", id), "dept_id", &dedup_ids(body.dept_ids))
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}
