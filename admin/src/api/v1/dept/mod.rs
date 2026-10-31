// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{check_len, would_cycle};
use crate::audit::{AuditDetail, Diff};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppJson, AppPath, ok};
use crate::hid;
use crate::models::{Admin, Dept};
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
pub(crate) fn validate_body(b: &DeptBody) -> Result<(), ApiError> {
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

#[apidoc::title("部门树")]
#[apidoc::desc("全量部门树（管理页用）。需要登录")]
#[apidoc::url("/api/v1/depts/tree")]
#[apidoc::method("GET")]
#[apidoc::tag("部门")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "array", desc = "部门树（顶层节点数组）", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "parent_id", ty = "string", required, desc = "父节点短串（0 = 根）"},
            {name = "name", ty = "string", required, desc = "部门名"},
            {name = "sort", ty = "int", desc = "排序（升序）"},
            {name = "leader", ty = "string", desc = "负责人（可为空串）"},
            {name = "phone", ty = "string", desc = "联系电话（可为空串）"},
            {name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用"},
            {name = "created_at", ty = "string", desc = "创建时间"},
            {name = "updated_at", ty = "string", desc = "更新时间"},
            {name = "children", ty = "array", desc = "子部门（同结构，递归；叶子为空数组 []）"},
        ])]
pub async fn tree(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("system:dept:list")?;
    let all = Dept::query()
        .order_by("sort ASC, id ASC")
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(build(&all, 0)))
}

#[apidoc::title("新建部门")]
#[apidoc::desc("父部门必须存在；名称超长 / 非法回 400")]
#[apidoc::url("/api/v1/depts")]
#[apidoc::method("POST")]
#[apidoc::tag("部门")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "parent_id", ty = "string", desc = "上级部门短串（也认裸数字）；空串 / null / 缺省 = 根（0）")]
#[apidoc::param(name = "name", ty = "string", required, desc = "部门名，最长 64 字符（落库前 trim）")]
#[apidoc::param(name = "sort", ty = "int", desc = "排序（升序），默认 0")]
#[apidoc::param(name = "leader", ty = "string", desc = "负责人，最长 64 字符")]
#[apidoc::param(name = "phone", ty = "string", desc = "联系电话，最长 20 字符")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "object", desc = "新建结果", children = [
            {name = "id", ty = "string", required, desc = "新部门对外 id（hashids 短串）"},
        ])]
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
    // 非超管只能在自己范围内建部门（否则能往看不见的部门树里塞节点）
    datascope::ensure_dept_in_scope(&auth, &state.db, body.parent_id).await?;

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

#[apidoc::title("编辑部门")]
#[apidoc::desc("父部门必须存在，且不能挂到自己的后代下（防环）")]
#[apidoc::url("/api/v1/depts/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("部门")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "部门对外 id（hashids 短串）")]
#[apidoc::param(name = "parent_id", ty = "string", desc = "上级部门短串（也认裸数字）；空串 / null / 缺省 = 根（0）")]
#[apidoc::param(name = "name", ty = "string", required, desc = "部门名，最长 64 字符（落库前 trim）")]
#[apidoc::param(name = "sort", ty = "int", desc = "排序（升序），默认 0")]
#[apidoc::param(name = "leader", ty = "string", desc = "负责人，最长 64 字符")]
#[apidoc::param(name = "phone", ty = "string", desc = "联系电话，最长 20 字符")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<DeptBody>,
) -> Result<(AuditDetail, Json<Value>), ApiError> {
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
    let old = d.clone(); // 变更摘要的旧值（下面就地改 d）

    // 数据范围闸：目标部门与新的父部门都必须在操作者范围内。否则只管 A 部的经理
    // 能把 B 部挂到 A 下，下一次请求他的「本部门及以下」就包含 B 及 B 下的人/日志。
    // 跨范围重挂只允许超管（ensure_dept_in_scope 对超管自然放行）。
    datascope::ensure_dept_in_scope(&auth, &state.db, d.id).await?;
    datascope::ensure_dept_in_scope(&auth, &state.db, body.parent_id).await?;

    d.parent_id = body.parent_id;
    d.name = body.name.trim().to_string();
    d.sort = body.sort;
    d.leader = body.leader;
    d.phone = body.phone;
    d.status = body.status;
    d.updated_at = now();
    // 变更摘要（项 8）：dept.phone 是明文列（只有 admin 的 email/phone 落库加密），照实记
    let changes = Diff::default()
        .field("parent_id", hid::enc(old.parent_id.max(0)), hid::enc(d.parent_id.max(0)))
        .field("name", old.name, d.name.clone())
        .field("sort", old.sort, d.sort)
        .field("leader", old.leader, d.leader.clone())
        .field("phone", old.phone, d.phone.clone())
        .field("status", old.status, d.status)
        .done();
    d.update(&state.db).await.map_err(ApiError::from)?;
    Ok((changes, ok(Value::Null)))
}

#[apidoc::title("删除部门")]
#[apidoc::desc("有子部门或挂着管理员时拒绝（400）")]
#[apidoc::url("/api/v1/depts/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("部门")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "部门对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dept:remove")?;
    let id = hid::dec(&id)?;
    // 存在才做范围校验：不存在的 id 保持原来的「200、删 0 行」幂等语义
    if let Some(d) = Dept::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
    {
        datascope::ensure_dept_in_scope(&auth, &state.db, d.id).await?;
    }
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
    // 先清角色-部门关联，避免留下指向已删除部门的悬挂脏行
    state
        .db
        .del_relations("role_dept", "dept_id", id)
        .await
        .map_err(ApiError::from)?;
    Dept::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .delete(&state.db)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}
