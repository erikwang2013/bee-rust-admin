// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{check_len, would_cycle};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppJson, AppPath, ok};
use crate::hid;
use crate::models::Menu;
use crate::relations::RelationsExt;
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use bee_orm::Model;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct MenuBody {
    #[serde(default, deserialize_with = "crate::hid::de_id")]
    pub parent_id: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub menu_type: String,
    #[serde(default)]
    pub perm: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub component: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub sort: i32,
    #[serde(default = "one")]
    pub visible: i8,
    #[serde(default = "one")]
    pub status: i8,
}

fn one() -> i8 {
    1
}

fn validate_type(t: &str) -> Result<(), ApiError> {
    if !matches!(t, "M" | "C" | "F") {
        return Err(ApiError::BadRequest("菜单类型只能是 M/C/F".into()));
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest("菜单名称不能为空".into()));
    }
    Ok(())
}

/// 长度上限与模型 `#[bee(len)]`、设计文档 §5.1 列宽一致（name 落库前会 trim）。
fn validate_body(b: &MenuBody) -> Result<(), ApiError> {
    validate_name(&b.name)?;
    validate_type(&b.menu_type)?;
    check_len("name", b.name.trim(), 64)?;
    check_len("perm", &b.perm, 128)?;
    check_len("path", &b.path, 128)?;
    check_len("component", &b.component, 128)?;
    check_len("icon", &b.icon, 64)
}

/// 父节点必须存在（0 = 根）。
async fn parent_exists(state: &AppState, parent_id: i64) -> Result<bool, ApiError> {
    if parent_id == 0 {
        return Ok(true);
    }
    Ok(Menu::query()
        .filter_eq("id", parent_id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .is_some())
}

async fn parent_map(state: &AppState) -> Result<HashMap<i64, i64>, ApiError> {
    Ok(Menu::query()
        .all(&state.db)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|m| (m.id, m.parent_id))
        .collect())
}

/// 全量树（含按钮节点，管理页要看）。
fn build(all: &[Menu], parent: i64) -> Vec<Value> {
    all.iter()
        .filter(|m| m.parent_id == parent)
        .map(|m| {
            let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
            v["children"] = json!(build(all, m.id));
            v
        })
        .collect()
}

#[apidoc::title("菜单树")]
#[apidoc::desc("全量树（含按钮节点，管理页要看）。需要登录")]
#[apidoc::url("/api/v1/menus/tree")]
#[apidoc::method("GET")]
#[apidoc::tag("菜单")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "array", desc = "菜单树（顶层节点数组；含按钮节点）", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "parent_id", ty = "string", required, desc = "父节点短串（0 = 顶层）"},
            {name = "name", ty = "string", required, desc = "菜单名"},
            {name = "type", ty = "string", required, desc = "类型：M 目录 / C 菜单 / F 按钮"},
            {name = "perm", ty = "string", desc = "权限码（M 目录为空串）"},
            {name = "path", ty = "string", desc = "前端路由路径（F 按钮为空串）"},
            {name = "component", ty = "string", desc = "前端组件路径（F 按钮为空串）"},
            {name = "icon", ty = "string", desc = "图标名（F 按钮为空串）"},
            {name = "sort", ty = "int", desc = "排序（升序）"},
            {name = "visible", ty = "int", desc = "是否显示：1 显示 / 0 隐藏"},
            {name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用"},
            {name = "created_at", ty = "string", desc = "创建时间"},
            {name = "updated_at", ty = "string", desc = "更新时间"},
            {name = "children", ty = "array", desc = "子节点（同结构，递归；叶子为空数组 []）"},
        ])]
pub async fn tree(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("system:menu:list")?;
    let all = Menu::query()
        .order_by("sort ASC, id ASC")
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(build(&all, 0)))
}

#[apidoc::title("新建菜单")]
#[apidoc::desc("父节点必须存在；类型/名称校验不通过回 400。非超管写的权限码必须是自己已有的（防就地造码提权）")]
#[apidoc::url("/api/v1/menus")]
#[apidoc::method("POST")]
#[apidoc::tag("菜单")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "parent_id", ty = "string", desc = "上级菜单短串（也认裸数字）；空串 / null / 缺省 = 顶层（0）")]
#[apidoc::param(name = "name", ty = "string", required, desc = "菜单名，最长 64 字符（落库前 trim）")]
#[apidoc::param(name = "type", ty = "string", required, desc = "类型：M 目录 / C 菜单 / F 按钮（其它值回 400）")]
#[apidoc::param(name = "perm", ty = "string", desc = "权限码，最长 128 字符（M 目录留空）")]
#[apidoc::param(name = "path", ty = "string", desc = "前端路由路径，最长 128 字符（M / C 用）")]
#[apidoc::param(name = "component", ty = "string", desc = "前端组件路径，最长 128 字符（C 用）")]
#[apidoc::param(name = "icon", ty = "string", desc = "图标名，最长 64 字符（M / C 用）")]
#[apidoc::param(name = "sort", ty = "int", desc = "排序（升序），默认 0")]
#[apidoc::param(name = "visible", ty = "int", desc = "是否显示：1 显示 / 0 隐藏，默认 1")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "object", desc = "新建结果", children = [
            {name = "id", ty = "string", required, desc = "新菜单对外 id（hashids 短串）"},
        ])]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<MenuBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:menu:add")?;
    validate_body(&body)?;
    // B5 写路径闸（非超管）：权限码只能写自己已有的（超管不受管）
    datascope::ensure_perm_writable(&auth, &body.perm)?;
    if !parent_exists(&state, body.parent_id).await? {
        return Err(ApiError::BadRequest("上级菜单不存在".into()));
    }

    let m = Menu {
        id: state.next_id()?,
        parent_id: body.parent_id,
        name: body.name.trim().to_string(),
        menu_type: body.menu_type,
        perm: body.perm,
        path: body.path,
        component: body.component,
        icon: body.icon,
        sort: body.sort,
        visible: body.visible,
        status: body.status,
        created_at: now(),
        updated_at: now(),
    };
    // id 已在上面发号，insert 就够（create 的读回是给自增主键用的）
    m.insert(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(json!({ "id": hid::enc(m.id) })))
}

#[apidoc::title("编辑菜单")]
#[apidoc::desc("父节点必须存在，且不能挂到自己的后代下（防环）。非超管写入的权限码必须是自己已有的")]
#[apidoc::url("/api/v1/menus/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("菜单")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "菜单对外 id（hashids 短串）")]
#[apidoc::param(name = "parent_id", ty = "string", desc = "上级菜单短串（也认裸数字）；空串 / null / 缺省 = 顶层（0）")]
#[apidoc::param(name = "name", ty = "string", required, desc = "菜单名，最长 64 字符（落库前 trim）")]
#[apidoc::param(name = "type", ty = "string", required, desc = "类型：M 目录 / C 菜单 / F 按钮（其它值回 400）")]
#[apidoc::param(name = "perm", ty = "string", desc = "权限码，最长 128 字符（M 目录留空）")]
#[apidoc::param(name = "path", ty = "string", desc = "前端路由路径，最长 128 字符（M / C 用）")]
#[apidoc::param(name = "component", ty = "string", desc = "前端组件路径，最长 128 字符（C 用）")]
#[apidoc::param(name = "icon", ty = "string", desc = "图标名，最长 64 字符（M / C 用）")]
#[apidoc::param(name = "sort", ty = "int", desc = "排序（升序），默认 0")]
#[apidoc::param(name = "visible", ty = "int", desc = "是否显示：1 显示 / 0 隐藏，默认 1")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<MenuBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:menu:edit")?;
    let id = hid::dec(&id)?;
    validate_body(&body)?;
    // B5 写路径闸（非超管）：权限码只能写自己已有的（超管不受管）
    datascope::ensure_perm_writable(&auth, &body.perm)?;
    if !parent_exists(&state, body.parent_id).await? {
        return Err(ApiError::BadRequest("上级菜单不存在".into()));
    }
    if would_cycle(&parent_map(&state).await?, id, body.parent_id) {
        return Err(ApiError::BadRequest("不能把菜单挂到自己的子节点下".into()));
    }

    let mut m = Menu::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    m.parent_id = body.parent_id;
    m.name = body.name.trim().to_string();
    m.menu_type = body.menu_type;
    m.perm = body.perm;
    m.path = body.path;
    m.component = body.component;
    m.icon = body.icon;
    m.sort = body.sort;
    m.visible = body.visible;
    m.status = body.status;
    m.updated_at = now();
    m.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("删除菜单")]
#[apidoc::desc("连带清理角色-菜单关联")]
#[apidoc::url("/api/v1/menus/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("菜单")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "菜单对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
// 400 是真实会返回的那两个（有子菜单 / 被角色引用）；**没有 404** ——
// 删一个不存在的 id 也回 200（delete 影响 0 行不报错）。这里原本写的是 404。
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:menu:remove")?;
    let id = hid::dec(&id)?;
    let children = Menu::query()
        .filter_eq("parent_id", id)
        .map_err(ApiError::from)?
        .count(&state.db)
        .await
        .map_err(ApiError::from)?;
    if children > 0 {
        return Err(ApiError::BadRequest("请先删除子菜单".into()));
    }
    let used = state.db.count_refs("role_menu", "menu_id", id).await.map_err(ApiError::from)?;
    if used > 0 {
        return Err(ApiError::BadRequest("该菜单已被角色引用".into()));
    }
    Menu::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .delete(&state.db)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}
