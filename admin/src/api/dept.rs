// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{check_len, would_cycle};
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, AppPath, ok};
use crate::hid;
use crate::models::{Admin, Dept};
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use bee_orm::Model;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct DeptBody {
    #[serde(default, deserialize_with = "crate::hid::de_id")]
    pub parent_id: i64,
    pub name: String,
    #[serde(default)]
    pub sort: i32,
    #[serde(default)]
    pub leader: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default = "one")]
    pub status: i8,
}

fn one() -> i8 {
    1
}

fn validate_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest("部门名称不能为空".into()));
    }
    Ok(())
}

/// 长度上限与模型 `#[bee(len)]`、设计文档 §5.1 列宽一致（name 落库前会 trim）。
fn validate_body(b: &DeptBody) -> Result<(), ApiError> {
    validate_name(&b.name)?;
    check_len("name", b.name.trim(), 64)?;
    check_len("leader", &b.leader, 64)?;
    check_len("contact", &b.phone, 20)
}

/// 父节点必须存在（0 = 根）。
async fn parent_exists(state: &AppState, parent_id: i64) -> Result<bool, ApiError> {
    if parent_id == 0 {
        return Ok(true);
    }
    Ok(Dept::query()
        .filter_eq("id", parent_id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .is_some())
}

async fn parent_map(state: &AppState) -> Result<HashMap<i64, i64>, ApiError> {
    Ok(Dept::query()
        .all(&state.db)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|d| (d.id, d.parent_id))
        .collect())
}

fn build(all: &[Dept], parent: i64) -> Vec<Value> {
    all.iter()
        .filter(|d| d.parent_id == parent)
        .map(|d| {
            let mut v = serde_json::to_value(d).unwrap_or(Value::Null);
            v["children"] = json!(build(all, d.id));
            v
        })
        .collect()
}

pub async fn tree(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("system:dept:list")?;
    let all = Dept::query()
        .order_by("sort ASC, id ASC")
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(build(&all, 0)))
}

pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<DeptBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dept:add")?;
    validate_body(&body)?;
    if !parent_exists(&state, body.parent_id).await? {
        return Err(ApiError::BadRequest("上级部门不存在".into()));
    }

    let d = Dept {
        id: state.next_id()?,
        parent_id: body.parent_id,
        name: body.name.trim().to_string(),
        sort: body.sort,
        leader: body.leader,
        phone: body.phone,
        status: body.status,
        created_at: now(),
        updated_at: now(),
    };
    // id 已在上面发号，insert 就够（create 的读回是给自增主键用的）
    d.insert(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(json!({ "id": hid::enc(d.id) })))
}

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<DeptBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dept:edit")?;
    let id = hid::dec(&id)?;
    validate_body(&body)?;
    if !parent_exists(&state, body.parent_id).await? {
        return Err(ApiError::BadRequest("上级部门不存在".into()));
    }
    if would_cycle(&parent_map(&state).await?, id, body.parent_id) {
        return Err(ApiError::BadRequest("不能把部门挂到自己的子部门下".into()));
    }

    let mut d = Dept::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    d.parent_id = body.parent_id;
    d.name = body.name.trim().to_string();
    d.sort = body.sort;
    d.leader = body.leader;
    d.phone = body.phone;
    d.status = body.status;
    d.updated_at = now();
    d.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dept:remove")?;
    let id = hid::dec(&id)?;
    let children = Dept::query()
        .filter_eq("parent_id", id)
        .map_err(ApiError::from)?
        .count(&state.db)
        .await
        .map_err(ApiError::from)?;
    if children > 0 {
        return Err(ApiError::BadRequest("请先删除子部门".into()));
    }
    let admins = Admin::query()
        .filter_eq("dept_id", id)
        .map_err(ApiError::from)?
        .count(&state.db)
        .await
        .map_err(ApiError::from)?;
    if admins > 0 {
        return Err(ApiError::BadRequest("该部门下还有管理员".into()));
    }
    Dept::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .delete(&state.db)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}
