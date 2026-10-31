// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{PagingExt, check_len, csv, dedup_ids, page_size};
use crate::audit::{AuditDetail, Diff, masked_changed};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::hid;
use crate::models::{Admin, Dept, Role};
use crate::relations::RelationsExt;
use crate::state::AppState;
use crate::util::{hash_password, now};
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use bee_orm::{Model, OrmError, QuerySet, Value as DbValue};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

mod manage;

pub use self::manage::*;

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

pub(crate) fn validate_username(u: &str) -> Result<(), ApiError> {
    let u = u.trim();
    if u.chars().count() < 3 || u.chars().count() > 64 {
        return Err(ApiError::BadRequest("用户名长度需 3-64 个字符".into()));
    }
    if !u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ApiError::BadRequest("用户名只能包含字母、数字、下划线".into()));
    }
    Ok(())
}

/// 密码复杂度（P6 最小加固）：**字符数** ≥ 8，且四类里至少两类。
///
/// 类别按 **ASCII** 划：小写 / 大写 / 数字，其余一律算「其它」（符号、汉字、emoji）。
/// 前端 `admin/web/src/password.ts` 必须同口径（`/[a-z]/`、`/[A-Z]/`、`/[0-9]/`、`/[^a-zA-Z0-9]/`）。
/// 只作用于**新设 / 新改**密码，登录路径不校验（存量密码照常能登）。
///
/// `label` 是消息里对密码的称呼（`"密码"` / `"新密码"`）。拼出来的 msg 必须与
/// `error.rs` 的 `ERR_TABLE` 逐字对上，否则 `err` 静默消失、前端译不了。
pub(crate) fn validate_password(p: &str, label: &str) -> Result<(), ApiError> {
    let classes = [
        p.chars().any(|c| c.is_ascii_lowercase()),
        p.chars().any(|c| c.is_ascii_uppercase()),
        p.chars().any(|c| c.is_ascii_digit()),
        p.chars().any(|c| !c.is_ascii_alphanumeric()),
    ]
    .into_iter()
    .filter(|hit| *hit)
    .count();
    if p.chars().count() < 8 || classes < 2 {
        return Err(ApiError::BadRequest(format!(
            "{label}至少 8 位，且需包含字母、数字、符号中的至少两类"
        )));
    }
    Ok(())
}

/// 长度上限是**明文**的业务上限（设计文档 §5.1），与列宽无关：email/phone 落库是
/// 密文，列宽按密文取（模型上的 `sql_type`）。明文上限是字符数，密文长度按**字节数**
/// 算，多字节字符会在 [`sealed`] 里被拦下（400），不会把明文截断着存。
pub(crate) fn validate_profile(
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
pub(crate) fn sealed(guard: &encryptable::guard::Guard, plain: &str) -> Result<String, ApiError> {
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

    // 本批 admin_id 一次查关联：列表页 10 行从 10 次查询降到 1 次，导出每批 5000 行同理。
    // ORDER BY admin_id, role_id 与逐行 `WHERE admin_id = ?` 走主键的返回顺序一致，
    // role_ids / role_names 的顺序保持不变。
    let ids_by_admin: HashMap<i64, Vec<i64>> = if rows.is_empty() {
        HashMap::new()
    } else {
        let placeholders = vec!["?"; rows.len()].join(", ");
        let sql = format!(
            "SELECT admin_id, role_id FROM admin_role WHERE admin_id IN ({placeholders}) \
             ORDER BY admin_id, role_id"
        );
        let params: Vec<DbValue> = rows.iter().map(|a| DbValue::from(a.id)).collect();
        let mut map: HashMap<i64, Vec<i64>> = HashMap::new();
        for row in state.db.query(&sql, &params).await.map_err(ApiError::from)? {
            if let (Some(admin_id), Some(role_id)) = (
                row.get("admin_id").and_then(|v| v.as_i64()),
                row.get("role_id").and_then(|v| v.as_i64()),
            ) {
                map.entry(admin_id).or_default().push(role_id);
            }
        }
        map
    };

    let mut list = Vec::with_capacity(rows.len());
    for a in rows {
        let ids = ids_by_admin.get(&a.id).cloned().unwrap_or_default();
        let names: Vec<String> = ids.iter().filter_map(|i| role_names.get(i).cloned()).collect();
        let mut v =
            serde_json::to_value(&a).map_err(|e| ApiError::internal(format!("序列化失败: {e}")))?;
        // email / phone 落库是密文，对外一律明文（列表 / 导出共用这条路径）
        crate::crypto::plain_json(&state.crypto, &mut v)?;
        // avatar 列存的是插件的 saved_path（内部地址），对外换成可用的 URL（无头像仍是空串）
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
#[apidoc::desc(
    "筛选 + 数据权限 + 分页，列表补 dept_name / role_names / role_ids（email / phone 出口解密为明文）"
)]
#[apidoc::url("/api/v1/admins")]
#[apidoc::method("GET")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1", mock = "1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100", mock = "10")]
#[apidoc::query(name = "username", ty = "string", desc = "用户名模糊匹配")]
#[apidoc::query(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用")]
#[apidoc::query(
    name = "dept_id",
    ty = "string",
    desc = "部门短串：**精确匹配该部门**（不展开子部门）"
)]
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
