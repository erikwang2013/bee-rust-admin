// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{check_len, would_cycle};
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, AppPath, ok};
use crate::models::{Menu, RoleMenu};
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct MenuBody {
    #[serde(default)]
    pub parent_id: u64,
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
async fn parent_exists(state: &AppState, parent_id: u64) -> Result<bool, ApiError> {
    if parent_id == 0 {
        return Ok(true);
    }
    Ok(Menu::query()
        .filter_eq("id", parent_id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .is_some())
}

async fn parent_map(state: &AppState) -> Result<HashMap<u64, u64>, ApiError> {
    Ok(Menu::query()
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|m| (m.id, m.parent_id))
        .collect())
}

/// 全量树（含按钮节点，管理页要看）。
fn build(all: &[Menu], parent: u64) -> Vec<Value> {
    all.iter()
        .filter(|m| m.parent_id == parent)
        .map(|m| {
            let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
            v["children"] = json!(build(all, m.id));
            v
        })
        .collect()
}

pub async fn tree(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("system:menu:list")?;
    let all = Menu::query()
        .order_by("sort ASC, id ASC")
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(build(&all, 0)))
}

pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<MenuBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:menu:add")?;
    validate_body(&body)?;
    if !parent_exists(&state, body.parent_id).await? {
        return Err(ApiError::BadRequest("上级菜单不存在".into()));
    }

    let mut m = Menu {
        id: 0,
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
    state.db.insert(&mut m).await.map_err(ApiError::from)?;
    Ok(ok(json!({ "id": m.id })))
}

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
    AppJson(body): AppJson<MenuBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:menu:edit")?;
    validate_body(&body)?;
    if !parent_exists(&state, body.parent_id).await? {
        return Err(ApiError::BadRequest("上级菜单不存在".into()));
    }
    if would_cycle(&parent_map(&state).await?, id, body.parent_id) {
        return Err(ApiError::BadRequest("不能把菜单挂到自己的子节点下".into()));
    }

    let mut m = Menu::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
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
    state.db.update(&m).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:menu:remove")?;
    let children = Menu::query()
        .filter_eq("parent_id", id)
        .map_err(ApiError::from)?
        .count(&state.db)
        .await
        .map_err(ApiError::from)?;
    if children > 0 {
        return Err(ApiError::BadRequest("请先删除子菜单".into()));
    }
    let used = RoleMenu::query()
        .filter_eq("menu_id", id)
        .map_err(ApiError::from)?
        .count(&state.db)
        .await
        .map_err(ApiError::from)?;
    if used > 0 {
        return Err(ApiError::BadRequest("该菜单已被角色引用".into()));
    }
    state.db.delete::<Menu>(id).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}
