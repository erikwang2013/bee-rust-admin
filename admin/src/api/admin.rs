// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{check_len, csv, dedup_ids, page_size};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::models::{Admin, Dept, Role};
use crate::state::AppState;
use crate::util::{hash_password, now};
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use bee_orm::{OrmError, QuerySet};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct AdminListQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub username: Option<String>,
    pub status: Option<i8>,
    pub dept_id: Option<u64>,
}

#[derive(Deserialize)]
pub struct AdminCreateBody {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub sex: i8,
    #[serde(default)]
    pub dept_id: u64,
    #[serde(default = "status_default")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
    #[serde(default)]
    pub role_ids: Vec<u64>,
}

#[derive(Deserialize)]
pub struct AdminUpdateBody {
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub sex: i8,
    #[serde(default)]
    pub dept_id: u64,
    #[serde(default = "status_default")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
    #[serde(default)]
    pub role_ids: Vec<u64>,
}

#[derive(Deserialize)]
pub struct StatusBody {
    pub status: i8,
}

#[derive(Deserialize)]
pub struct PasswordBody {
    pub password: String,
}

#[derive(Deserialize)]
pub struct RolesBody {
    pub role_ids: Vec<u64>,
}

fn status_default() -> i8 {
    1
}

fn validate_username(u: &str) -> Result<(), ApiError> {
    let u = u.trim();
    if u.chars().count() < 3 || u.chars().count() > 64 {
        return Err(ApiError::BadRequest("用户名长度需 3-64 个字符".into()));
    }
    if !u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ApiError::BadRequest("用户名只能包含字母、数字、下划线".into()));
    }
    Ok(())
}

fn validate_password(p: &str) -> Result<(), ApiError> {
    if p.chars().count() < 6 {
        return Err(ApiError::BadRequest("密码至少 6 位".into()));
    }
    Ok(())
}

/// 长度上限与模型 `#[bee(len)]`、设计文档 §5.1 列宽一致。
fn validate_profile(
    nickname: &str,
    email: &str,
    phone: &str,
    remark: &str,
) -> Result<(), ApiError> {
    check_len("昵称", nickname, 64)?;
    check_len("邮箱", email, 128)?;
    check_len("手机号", phone, 20)?;
    check_len("备注", remark, 255)
}

/// 列表与导出共用的筛选 + 数据权限。
async fn filtered(
    q: &AdminListQuery,
    auth: &Auth,
    state: &AppState,
) -> Result<QuerySet<Admin>, ApiError> {
    let mut qs = Admin::query();
    if let Some(u) = q.username.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("username", u.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    if let Some(d) = q.dept_id {
        qs = qs.filter_eq("dept_id", d).map_err(ApiError::from)?;
    }
    let scope = datascope::resolve(auth, &state.db).await?;
    Ok(datascope::apply(qs, &scope, "dept_id", "id"))
}

/// 补 dept_name / role_names / role_ids / is_super（列表与导出共用）。
async fn decorate(state: &AppState, rows: Vec<Admin>) -> Result<Vec<Value>, ApiError> {
    let dept_names: HashMap<u64, String> = Dept::query()
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|d| (d.id, d.name))
        .collect();
    let role_names: HashMap<u64, String> = Role::query()
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|r| (r.id, r.name))
        .collect();

    let mut list = Vec::with_capacity(rows.len());
    for a in rows {
        // ponytail: 每行一次关联查询（页 10 行 = 11 次查询）；行数大改批量查
        let ids = state
            .db
            .get_relations("admin_role", ("admin_id", a.id), "role_id")
            .await
            .map_err(ApiError::from)?;
        let names: Vec<String> = ids.iter().filter_map(|i| role_names.get(i).cloned()).collect();
        let mut v = serde_json::to_value(&a)
            .map_err(|e| ApiError::internal(format!("序列化失败: {e}")))?;
        v["dept_name"] = json!(dept_names.get(&a.dept_id).cloned().unwrap_or_default());
        v["role_names"] = json!(names);
        v["role_ids"] = json!(ids);
        v["is_super"] = json!(a.is_super == 1); // 前端契约：bool
        list.push(v);
    }
    Ok(list)
}

/// 列表：筛选 + 数据权限 + 分页；补 dept_name / role_names。
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<AdminListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:list")?;
    let (page, size) = page_size(q.page, q.size);
    let (rows, total) = filtered(&q, &auth, &state)
        .await?
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    let list = decorate(&state, rows).await?;
    Ok(ok(json!({ "list": list, "total": total })))
}

fn export_header() -> Vec<String> {
    ["ID", "用户名", "昵称", "部门", "角色", "邮箱", "手机号", "状态", "最后登录", "创建时间"]
        .map(String::from)
        .to_vec()
}

/// 取一批导出数据（keyset：`id < last`，首页不带该条件），补字段/渲染与旧的一次性实现一致。
async fn export_batch(
    state: &AppState,
    base: &QuerySet<Admin>,
    last: Option<u64>,
) -> Result<Vec<(u64, String)>, ApiError> {
    let mut qs = base.clone().order_by("id DESC").limit(csv::BATCH);
    if let Some(last) = last {
        qs = qs.filter_raw("id < ?", &[last]);
    }
    let rows = qs.fetch_all(&state.db).await.map_err(ApiError::from)?;
    Ok(decorate(state, rows)
        .await?
        .into_iter()
        .map(|v| {
            let id = v["id"].as_u64().unwrap_or(0);
            (
                id,
                csv::row(&[
                    csv::jcell(&v["id"]),
                    csv::jcell(&v["username"]),
                    csv::jcell(&v["nickname"]),
                    csv::jcell(&v["dept_name"]),
                    csv::jcell(&v["role_names"]),
                    csv::jcell(&v["email"]),
                    csv::jcell(&v["phone"]),
                    if v["status"].as_i64() == Some(1) { "启用".into() } else { "禁用".into() },
                    csv::jcell(&v["last_login_at"]),
                    csv::jcell(&v["created_at"]),
                ]),
            )
        })
        .collect())
}

/// 导出当前筛选结果（不含分页）；鉴权复用 list 权限码。
/// 流式：筛选 + 数据权限与 list 共用 `filtered()`，按 keyset 分批取（B6），
/// 内存只与一批（含该批的 decorate）成正比。
pub async fn export(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<AdminListQuery>,
) -> Result<Response, ApiError> {
    auth.require("system:admin:list")?;
    let base = filtered(&q, &auth, &state).await?;
    csv::streamed(format!("admins-{}.csv", csv::today()), export_header(), move |last| {
        let state = state.clone();
        let base = base.clone();
        async move { export_batch(&state, &base, last).await }
    })
    .await
}

pub async fn detail(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:list")?;
    let a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    datascope::ensure_admin_in_scope(&auth, &state.db, &a).await?;
    let role_ids = state
        .db
        .get_relations("admin_role", ("admin_id", id), "role_id")
        .await
        .map_err(ApiError::from)?;
    let mut v = serde_json::to_value(&a).map_err(|e| ApiError::internal(format!("序列化失败: {e}")))?;
    v["role_ids"] = json!(role_ids);
    v["is_super"] = json!(a.is_super == 1); // 前端契约：bool
    Ok(ok(v))
}

pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<AdminCreateBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:add")?;
    validate_username(&body.username)?;
    validate_password(&body.password)?;
    validate_profile(&body.nickname, &body.email, &body.phone, &body.remark)?;
    // 建人也要在范围内：部门与角色都不能超出操作者自己（B5）
    datascope::ensure_dept_in_scope(&auth, &state.db, body.dept_id).await?;
    datascope::ensure_roles_grantable(&auth, &state.db, &body.role_ids).await?;

    let mut a = Admin {
        id: 0,
        username: body.username.trim().to_string(),
        password: hash_password(&body.password),
        nickname: body.nickname,
        email: body.email,
        phone: body.phone,
        sex: body.sex,
        avatar: String::new(),
        dept_id: body.dept_id,
        status: body.status,
        is_super: 0,
        token_version: 0,
        last_login_at: None,
        last_login_ip: String::new(),
        remark: body.remark,
        created_at: now(),
        updated_at: now(),
    };
    match state.db.insert(&mut a).await {
        Ok(_) => {}
        Err(OrmError::DuplicateKey(_)) => return Err(ApiError::BadRequest("用户名已存在".into())),
        Err(e) => return Err(e.into()),
    }
    state
        .db
        .set_relations("admin_role", ("admin_id", a.id), "role_id", &dedup_ids(body.role_ids))
        .await
        .map_err(ApiError::from)?;

    Ok(ok(json!({ "id": a.id })))
}

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
    AppJson(body): AppJson<AdminUpdateBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    validate_profile(&body.nickname, &body.email, &body.phone, &body.remark)?;
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    // 本接口同样能改状态，必须走 set_status 的保护，否则可以绕过它禁用超管/自己
    let new_status = if body.status == 0 { 0 } else { 1 };
    if new_status != a.status {
        if id == auth.admin.id {
            return Err(ApiError::BadRequest("不能修改自己的状态".into()));
        }
        if a.is_super == 1 {
            return Err(ApiError::BadRequest("不能修改超级管理员的状态".into()));
        }
        if new_status != 1 {
            a.token_version += 1; // 禁用即踢下线
        }
    }

    // 范围与授权闸门（B5）：目标、目标部门、要授的角色三者都不能超出操作者。
    // 放在状态保护之后 —— 「不能改超管/自己状态」是更明确的 400（既有契约，
    // api_flow_test 钉着），范围判定只在它之后兜底。
    datascope::ensure_admin_in_scope(&auth, &state.db, &a).await?;
    datascope::ensure_dept_in_scope(&auth, &state.db, body.dept_id).await?;
    datascope::ensure_roles_added_grantable(&auth, &state.db, id, &body.role_ids).await?;

    a.nickname = body.nickname;
    a.email = body.email;
    a.phone = body.phone;
    a.sex = body.sex;
    a.dept_id = body.dept_id;
    a.status = new_status;
    a.remark = body.remark;
    a.updated_at = now();
    state.db.update(&a).await.map_err(ApiError::from)?;
    state
        .db
        .set_relations("admin_role", ("admin_id", id), "role_id", &dedup_ids(body.role_ids))
        .await
        .map_err(ApiError::from)?;

    Ok(ok(Value::Null))
}

pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:remove")?;
    if id == auth.admin.id {
        return Err(ApiError::BadRequest("不能删除自己".into()));
    }
    let a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if a.is_super == 1 {
        return Err(ApiError::BadRequest("不能删除超级管理员".into()));
    }
    datascope::ensure_admin_in_scope(&auth, &state.db, &a).await?;
    state
        .db
        .del_relations("admin_role", "admin_id", id)
        .await
        .map_err(ApiError::from)?;
    state.db.delete::<Admin>(id).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

pub async fn set_status(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
    AppJson(body): AppJson<StatusBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    if id == auth.admin.id {
        return Err(ApiError::BadRequest("不能修改自己的状态".into()));
    }
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if a.is_super == 1 {
        return Err(ApiError::BadRequest("不能修改超级管理员的状态".into()));
    }
    datascope::ensure_admin_in_scope(&auth, &state.db, &a).await?;
    a.status = if body.status == 0 { 0 } else { 1 };
    if a.status != 1 {
        a.token_version += 1; // 禁用即踢下线
    }
    a.updated_at = now();
    state.db.update(&a).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

pub async fn reset_password(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
    AppJson(body): AppJson<PasswordBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:resetPwd")?;
    validate_password(&body.password)?;
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if a.is_super == 1 && id != auth.admin.id {
        return Err(ApiError::BadRequest("不能重置其他超级管理员的密码".into()));
    }
    datascope::ensure_admin_in_scope(&auth, &state.db, &a).await?;
    a.password = hash_password(&body.password);
    a.token_version += 1; // 旧 token 立即失效
    a.updated_at = now();
    state.db.update(&a).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

pub async fn set_roles(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
    AppJson(body): AppJson<RolesBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    let target = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    datascope::ensure_admin_in_scope(&auth, &state.db, &target).await?;
    // 新增的角色不能比自己宽（自己给自己授 data_scope=1 就是自我提权）；
    // 已挂着的照旧保留，别把「改个昵称」也一起拒了
    datascope::ensure_roles_added_grantable(&auth, &state.db, target.id, &body.role_ids).await?;
    state
        .db
        .set_relations("admin_role", ("admin_id", id), "role_id", &dedup_ids(body.role_ids))
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}
