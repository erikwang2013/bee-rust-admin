// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{PagingExt, check_len, csv, dedup_ids, page_size};
use crate::auth::Auth;
use crate::datascope;
use crate::hid;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::models::{Admin, Dept, Role};
use crate::state::AppState;
use crate::util::{hash_password, now};
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use bee_orm::{Model, OrmError, QuerySet};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use crate::relations::RelationsExt;

#[derive(Deserialize, Clone)]
pub struct AdminListQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub username: Option<String>,
    pub status: Option<i8>,
    #[serde(default, deserialize_with = "crate::hid::de_opt_id")]
    pub dept_id: Option<i64>,
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
    #[serde(default, deserialize_with = "crate::hid::de_id")]
    pub dept_id: i64,
    #[serde(default = "status_default")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
    #[serde(default, deserialize_with = "crate::hid::de_vec_id")]
    pub role_ids: Vec<i64>,
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
    #[serde(default, deserialize_with = "crate::hid::de_id")]
    pub dept_id: i64,
    #[serde(default = "status_default")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
    #[serde(default, deserialize_with = "crate::hid::de_vec_id")]
    pub role_ids: Vec<i64>,
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
    #[serde(deserialize_with = "crate::hid::de_vec_id")]
    pub role_ids: Vec<i64>,
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

/// 长度上限是**明文**的业务上限（设计文档 §5.1），与列宽无关：email/phone 落库是
/// 密文，列宽按密文取（模型上的 `sql_type`）。明文上限是字符数，密文长度按**字节数**
/// 算，多字节字符会在 [`sealed`] 里被拦下（400），不会把明文截断着存。
fn validate_profile(
    nickname: &str,
    email: &str,
    phone: &str,
    remark: &str,
) -> Result<(), ApiError> {
    check_len("nickname", nickname, 64)?;
    check_len("email", email, 128)?;
    check_len("phone", phone, 20)?;
    check_len("remark", remark, 255)
}

/// 明文 → 落库密文（`crate::crypto`）。写 email/phone 的地方统一走它，别处不许直接赋值。
fn sealed(guard: &encryptable::guard::Guard, plain: &str) -> Result<String, ApiError> {
    crate::crypto::write(guard, plain).map_err(ApiError::BadRequest)
}

/// 列表与导出共用的筛选条件（不含数据权限）。
fn filtered_query(q: &AdminListQuery) -> Result<QuerySet<Admin>, ApiError> {
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
    Ok(qs)
}

/// 列表与导出共用的筛选 + 数据权限。
async fn filtered(
    q: &AdminListQuery,
    auth: &Auth,
    state: &AppState,
) -> Result<QuerySet<Admin>, ApiError> {
    let scope = datascope::resolve(auth, &state.db).await?;
    Ok(datascope::apply(filtered_query(q)?, &scope, "dept_id", "id"))
}

/// 补 dept_name / role_names / role_ids / is_super（列表与导出共用）。
async fn decorate(state: &AppState, rows: Vec<Admin>) -> Result<Vec<Value>, ApiError> {
    let dept_names: HashMap<i64, String> = Dept::query()
        .all(&state.db)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|d| (d.id, d.name))
        .collect();
    let role_names: HashMap<i64, String> = Role::query()
        .all(&state.db)
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
        // email / phone 落库是密文，对外一律明文（列表 / 导出共用这条路径）
        crate::crypto::plain_json(&state.crypto, &mut v)?;
        // avatar 列存的是插件的 savedPath（内部地址），对外换成可用的 URL（无头像仍是空串）
        v["avatar"] = json!(crate::api::avatar::public_avatar(&a.avatar, a.id));
        v["dept_name"] = json!(dept_names.get(&a.dept_id).cloned().unwrap_or_default());
        v["role_names"] = json!(names);
        v["role_ids"] = json!(hid::enc_vec(&ids));
        v["is_super"] = json!(a.is_super == 1); // 前端契约：bool
        list.push(v);
    }
    Ok(list)
}

/// 列表：筛选 + 数据权限 + 分页；补 dept_name / role_names。
#[apidoc::title("管理员列表")]
#[apidoc::desc("筛选 + 数据权限 + 分页，列表补 dept_name / role_names / role_ids（email / phone 出口解密为明文）")]
#[apidoc::url("/api/v1/admins")]
#[apidoc::method("GET")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1", mock = "1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100", mock = "10")]
#[apidoc::query(name = "username", ty = "string", desc = "用户名模糊匹配")]
#[apidoc::query(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用")]
#[apidoc::query(name = "dept_id", ty = "string", desc = "部门短串（含子部门，受数据权限约束）")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "分页结果", children = [
            {name = "list", ty = "array", required, desc = "管理员列表", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "username", ty = "string", required, desc = "用户名"},
            {name = "nickname", ty = "string", desc = "昵称"},
            {name = "avatar", ty = "string", desc = "头像地址（无头像为空串）"},
            {name = "email", ty = "string", desc = "邮箱（库里密文，出口明文）"},
            {name = "phone", ty = "string", desc = "手机号（同上）"},
            {name = "sex", ty = "int", desc = "性别：0 未知 / 1 男 / 2 女"},
            {name = "dept_id", ty = "string", desc = "部门短串（0 = 无部门）"},
            {name = "dept_name", ty = "string", desc = "部门名（列表补的）"},
            {name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用"},
            {name = "is_super", ty = "bool", desc = "是否超级管理员"},
            {name = "remark", ty = "string", desc = "备注"},
            {name = "last_login_at", ty = "string", desc = "最后登录时间（空 = 从未登录）"},
            {name = "last_login_ip", ty = "string", desc = "最后登录 IP"},
            {name = "created_at", ty = "string", desc = "创建时间"},
            {name = "updated_at", ty = "string", desc = "更新时间"},
            {name = "role_ids", ty = "array", desc = "角色短串数组"},
            {name = "role_names", ty = "array", desc = "角色名数组"},
        ]},
            {name = "total", ty = "int", required, desc = "筛选后的总条数"},
        ])]
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
/// `QuerySet` 不可克隆，所以每批用筛选条件 + 已解析的数据范围重建（`scope` 只解析一次）。
async fn export_batch(
    state: &AppState,
    q: &AdminListQuery,
    scope: &datascope::DataScope,
    last: Option<i64>,
) -> Result<Vec<(i64, String)>, ApiError> {
    let mut qs = datascope::apply(filtered_query(q)?, scope, "dept_id", "id")
        .order_by("id DESC")
        .limit(csv::BATCH);
    if let Some(last) = last {
        qs = qs.filter_lt("id", last).map_err(ApiError::from)?;
    }
    let rows = qs.all(&state.db).await.map_err(ApiError::from)?;
    // keyset 用原始 i64；CSV 里呈现的 id 是 decorate 序列化出的短串（与列表一致）
    let ids: Vec<i64> = rows.iter().map(|a| a.id).collect();
    Ok(ids
        .into_iter()
        .zip(decorate(state, rows).await?)
        .map(|(id, v)| {
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
#[apidoc::title("导出管理员 CSV")]
#[apidoc::desc("导出当前筛选结果（不含分页，流式，列与列表一致）；鉴权复用列表权限码")]
#[apidoc::url("/api/v1/admins/export")]
#[apidoc::method("GET")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "username", ty = "string", desc = "用户名模糊匹配")]
#[apidoc::query(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用")]
#[apidoc::query(name = "dept_id", ty = "string", desc = "部门短串")]
#[apidoc::response_status("200")]
#[apidoc::not_debug]
pub async fn export(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<AdminListQuery>,
) -> Result<Response, ApiError> {
    auth.require("system:admin:list")?;
    let scope = datascope::resolve(&auth, &state.db).await?;
    csv::streamed(format!("admins-{}.csv", csv::today()), export_header(), move |last| {
        let state = state.clone();
        let q = q.clone();
        let scope = scope.clone();
        async move { export_batch(&state, &q, &scope, last).await }
    })
    .await
}

#[apidoc::title("管理员详情")]
#[apidoc::desc("编辑弹窗回显：管理员字段 + role_ids（email / phone 明文），目标超出数据范围时 403")]
#[apidoc::url("/api/v1/admins/{id}")]
#[apidoc::method("GET")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
#[apidoc::returned(name = "data", ty = "object", desc = "管理员对象", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "username", ty = "string", required, desc = "用户名"},
            {name = "nickname", ty = "string", desc = "昵称"},
            {name = "avatar", ty = "string", desc = "头像地址（无头像为空串）"},
            {name = "email", ty = "string", desc = "邮箱（库里密文，出口明文）"},
            {name = "phone", ty = "string", desc = "手机号（同上）"},
            {name = "sex", ty = "int", desc = "性别：0 未知 / 1 男 / 2 女"},
            {name = "dept_id", ty = "string", desc = "部门短串（0 = 无部门）"},
            {name = "dept_name", ty = "string", desc = "部门名（列表补的）"},
            {name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用"},
            {name = "is_super", ty = "bool", desc = "是否超级管理员"},
            {name = "remark", ty = "string", desc = "备注"},
            {name = "last_login_at", ty = "string", desc = "最后登录时间（空 = 从未登录）"},
            {name = "last_login_ip", ty = "string", desc = "最后登录 IP"},
            {name = "created_at", ty = "string", desc = "创建时间"},
            {name = "updated_at", ty = "string", desc = "更新时间"},
            {name = "role_ids", ty = "array", desc = "角色短串数组"},
            {name = "role_names", ty = "array", desc = "角色名数组"},
        ])]
pub async fn detail(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:list")?;
    let id = hid::dec(&id)?;
    let a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
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
    // email / phone 落库是密文，详情/编辑回显要明文（前端无感）
    crate::crypto::plain_json(&state.crypto, &mut v)?;
    // 同列表：avatar 列存的是 savedPath，回给前端的是 URL
    v["avatar"] = json!(crate::api::avatar::public_avatar(&a.avatar, a.id));
    v["role_ids"] = json!(hid::enc_vec(&role_ids));
    v["is_super"] = json!(a.is_super == 1); // 前端契约：bool
    Ok(ok(v))
}

/// 用户名重复 → 该接口的专属文案（`admin.username_taken`）。
/// 唯一键冲突的识别在 `ApiError::from` 里按 MySQL 报错文本做（上游 `OrmError`
/// 没有独立变体），冲突是它唯一映射成 `BadRequest` 的情况，其余是 `NotFound` / `Internal`。
fn username_taken(e: OrmError) -> ApiError {
    match ApiError::from(e) {
        ApiError::BadRequest(_) => ApiError::BadRequest("用户名已存在".into()),
        other => other,
    }
}

#[apidoc::title("新建管理员")]
#[apidoc::desc("用户名 3-64 字符（字母/数字/下划线）；部门与角色都不能超出操作者的数据范围")]
#[apidoc::url("/api/v1/admins")]
#[apidoc::method("POST")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "username", ty = "string", required, desc = "用户名，3-64 个字符（字母 / 数字 / 下划线，唯一）")]
#[apidoc::param(name = "password", ty = "string", required, desc = "登录密码，至少 6 位")]
#[apidoc::param(name = "nickname", ty = "string", desc = "昵称，最长 64 字符")]
#[apidoc::param(name = "email", ty = "string", desc = "邮箱（落库加密），最长 128 字符")]
#[apidoc::param(name = "phone", ty = "string", desc = "手机号（落库加密），最长 20 字符")]
#[apidoc::param(name = "sex", ty = "int", desc = "性别：0 未知 / 1 男 / 2 女")]
#[apidoc::param(name = "dept_id", ty = "string", desc = "部门短串（0 = 无部门）")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注，最长 255 字符")]
#[apidoc::param(name = "role_ids", ty = "array", desc = "角色短串数组")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "object", desc = "新建结果", children = [
            {name = "id", ty = "string", required, desc = "新管理员对外 id（hashids 短串）"},
        ])]
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

    let a = Admin {
        id: state.next_id()?,
        username: body.username.trim().to_string(),
        password: hash_password(&body.password),
        nickname: body.nickname,
        email: sealed(&state.crypto, &body.email)?,
        phone: sealed(&state.crypto, &body.phone)?,
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
    // id 已在上面发号，insert 就够（create 的读回是给自增主键用的）
    a.insert(&state.db).await.map_err(username_taken)?;
    state
        .db
        .set_relations("admin_role", ("admin_id", a.id), "role_id", &dedup_ids(body.role_ids))
        .await
        .map_err(ApiError::from)?;

    Ok(ok(json!({ "id": hid::enc(a.id) })))
}

#[apidoc::title("编辑管理员")]
#[apidoc::desc("本接口同样能改状态：禁用即踢下线；不能改自己 / 超管的状态，目标与要授的角色都不能超出操作者范围")]
#[apidoc::url("/api/v1/admins/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::param(name = "nickname", ty = "string", desc = "昵称，最长 64 字符")]
#[apidoc::param(name = "email", ty = "string", desc = "邮箱（落库加密），最长 128 字符")]
#[apidoc::param(name = "phone", ty = "string", desc = "手机号（落库加密），最长 20 字符")]
#[apidoc::param(name = "sex", ty = "int", desc = "性别：0 未知 / 1 男 / 2 女")]
#[apidoc::param(name = "dept_id", ty = "string", desc = "部门短串（0 = 无部门）")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注，最长 255 字符")]
#[apidoc::param(name = "role_ids", ty = "array", desc = "角色短串数组（整批覆盖）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<AdminUpdateBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    let id = hid::dec(&id)?;
    validate_profile(&body.nickname, &body.email, &body.phone, &body.remark)?;
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
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
    a.email = sealed(&state.crypto, &body.email)?;
    a.phone = sealed(&state.crypto, &body.phone)?;
    a.sex = body.sex;
    a.dept_id = body.dept_id;
    a.status = new_status;
    a.remark = body.remark;
    a.updated_at = now();
    a.update(&state.db).await.map_err(ApiError::from)?;
    state
        .db
        .set_relations("admin_role", ("admin_id", id), "role_id", &dedup_ids(body.role_ids))
        .await
        .map_err(ApiError::from)?;

    Ok(ok(Value::Null))
}

#[apidoc::title("删除管理员")]
#[apidoc::desc("连带删管理员-角色关联；不能删自己，也不能删超管")]
#[apidoc::url("/api/v1/admins/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:remove")?;
    let id = hid::dec(&id)?;
    if id == auth.admin.id {
        return Err(ApiError::BadRequest("不能删除自己".into()));
    }
    let a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
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
    a.delete(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("启用 / 禁用管理员")]
#[apidoc::desc("禁用即踢下线（token_version 自增）；不能改自己或超管的状态")]
#[apidoc::url("/api/v1/admins/{id}/status")]
#[apidoc::method("PUT")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::param(name = "status", ty = "int", required, desc = "状态：1 启用 / 0 禁用")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn set_status(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<StatusBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    let id = hid::dec(&id)?;
    if id == auth.admin.id {
        return Err(ApiError::BadRequest("不能修改自己的状态".into()));
    }
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
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
    a.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("重置管理员密码")]
#[apidoc::desc("新密码至少 6 位；旧 token 立即失效（token_version 自增）。不能重置其他超管的密码")]
#[apidoc::url("/api/v1/admins/{id}/password")]
#[apidoc::method("PUT")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::param(name = "password", ty = "string", required, desc = "新密码，至少 6 位")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn reset_password(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<PasswordBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:resetPwd")?;
    let id = hid::dec(&id)?;
    validate_password(&body.password)?;
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
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
    a.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("设置管理员角色")]
#[apidoc::desc("整批覆盖；新增的角色不能超出操作者的数据范围（防自我提权）")]
#[apidoc::url("/api/v1/admins/{id}/roles")]
#[apidoc::method("PUT")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::param(name = "role_ids", ty = "array", required, desc = "角色短串数组（整批覆盖，重复项自动去重）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn set_roles(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<RolesBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    let id = hid::dec(&id)?;
    let target = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
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
