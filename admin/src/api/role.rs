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
