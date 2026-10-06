// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 字典管理（v1.5.0）：字典类型 + 字典项。
//! 除 `GET /dicts/{code}/items`（下拉数据源，登录即可）外都要权限码。
use crate::api::{check_len, csv, page_size};
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::models::{DictItem, DictType};
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use bee_orm::OrmError;
use serde::Deserialize;
use serde_json::{Value, json};

fn one() -> i8 {
    1
}

/// 字典编码：小写字母/数字/下划线，2~64 位。它既是字典项的关联键，
/// 又是接口路径段（`/dicts/{code}/items`），限死字符集就省掉了两头转义。
fn valid_code(code: &str) -> bool {
    let n = code.chars().count();
    (2..=64).contains(&n)
        && code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn check_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest("字典名称不能为空".into()));
    }
    check_len("name", name.trim(), 64)
}

fn check_value(value: &str) -> Result<(), ApiError> {
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

fn filtered_types(q: &DictTypeQuery) -> Result<bee_orm::QuerySet<DictType>, ApiError> {
    let mut qs = DictType::query();
    if let Some(n) = q.name.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("name", n.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    Ok(qs)
}

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

    let mut t = DictType {
        id: 0,
        name: body.name.trim().to_string(),
        code: code.to_string(),
        status: body.status,
        remark: body.remark,
        created_at: now(),
        updated_at: now(),
    };
    match state.db.insert(&mut t).await {
        Ok(_) => {}
        Err(OrmError::DuplicateKey(_)) => return Err(ApiError::BadRequest("字典编码已存在".into())),
        Err(e) => return Err(e.into()),
    }
    Ok(ok(json!({ "id": t.id })))
}

pub async fn type_update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
    AppJson(body): AppJson<DictTypeUpdate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:edit")?;
    check_name(&body.name)?;
    check_len("remark", &body.remark, 255)?;

    let mut t = DictType::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    t.name = body.name.trim().to_string();
    t.status = body.status;
    t.remark = body.remark;
    t.updated_at = now();
    match state.db.update(&t).await {
        Ok(_) => {}
        Err(OrmError::DuplicateKey(_)) => return Err(ApiError::BadRequest("字典编码已存在".into())),
        Err(e) => return Err(e.into()),
    }
    Ok(ok(Value::Null))
}

/// 删类型连带删它的字典项，同一事务：中途失败回滚，不留「类型没了项还在」的孤儿行。
/// 项数按字典量级（一条几到几十个）逐条删够用。
pub async fn type_remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:remove")?;
    let t = DictType::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    let items = DictItem::query()
        .filter_eq("type_code", &t.code)
        .map_err(ApiError::from)?
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;

    let mut tx = state.db.begin().await.map_err(ApiError::from)?;
    for it in &items {
        tx.delete::<DictItem>(it.id).await.map_err(ApiError::from)?;
    }
    tx.delete::<DictType>(t.id).await.map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(ok(json!({ "items_deleted": items.len() })))
}

/// 类型必须存在：不建外键，至少别让手滑的 `type_code` 造出孤儿项（下拉里永远看不到）。
async fn type_exists(state: &AppState, code: &str) -> Result<bool, ApiError> {
    Ok(DictType::query()
        .filter_eq("code", code)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .is_some())
}

/// 下拉数据源：只认登录，不要求 `system:dict:list`（任何登录用户都要能取字典）。
/// 只回启用项，按 `sort, id` 排序；字段只留前端要的 `label` / `value`。
///
/// 类型不存在回 404（拼错 code 的人该看到「类型不存在」而不是一个空下拉，
/// 分不清「没配数据」和「写错了」）；类型在但没启用项回 200 + `[]`——那是合法状态。
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
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|i| json!({ "label": i.label, "value": i.value }))
        .collect();
    Ok(ok(out))
}

// ── 字典项 ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DictItemQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub type_code: Option<String>,
    pub label: Option<String>,
    pub status: Option<i8>,
}

#[derive(Deserialize)]
pub struct DictItemCreate {
    pub type_code: String,
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub sort: i32,
    #[serde(default = "one")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
}

/// 与类型同理：`type_code` 是关联键，更新体里没有它（前端提交了也忽略）。
#[derive(Deserialize)]
pub struct DictItemUpdate {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub sort: i32,
    #[serde(default = "one")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
}

fn check_label(label: &str) -> Result<(), ApiError> {
    if label.trim().is_empty() {
        return Err(ApiError::BadRequest("字典标签不能为空".into()));
    }
    check_len("label", label.trim(), 64)
}

fn filtered_items(q: &DictItemQuery) -> Result<bee_orm::QuerySet<DictItem>, ApiError> {
    let mut qs = DictItem::query();
    if let Some(c) = q.type_code.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_eq("type_code", c.trim()).map_err(ApiError::from)?;
    }
    if let Some(l) = q.label.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("label", l.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    Ok(qs)
}

pub async fn item_list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<DictItemQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:list")?;
    let (page, size) = page_size(q.page, q.size);
    let (rows, total) = filtered_items(&q)?
        // 与下拉同序：管理页看到的顺序就是前端下拉的顺序
        .order_by("sort ASC, id ASC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

pub async fn item_create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<DictItemCreate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:add")?;
    check_label(&body.label)?;
    check_value(&body.value)?;
    check_len("remark", &body.remark, 255)?;
    let type_code = body.type_code.trim();
    if !type_exists(&state, type_code).await? {
        return Err(ApiError::BadRequest("字典类型不存在".into()));
    }

    let mut it = DictItem {
        id: 0,
        type_code: type_code.to_string(),
        label: body.label.trim().to_string(),
        value: body.value.trim().to_string(),
        sort: body.sort,
        status: body.status,
        remark: body.remark,
        created_at: now(),
        updated_at: now(),
    };
    match state.db.insert(&mut it).await {
        Ok(_) => {}
        Err(OrmError::DuplicateKey(_)) => {
            return Err(ApiError::BadRequest("该类型下字典值已存在".into()));
        }
        Err(e) => return Err(e.into()),
    }
    Ok(ok(json!({ "id": it.id })))
}

pub async fn item_update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
    AppJson(body): AppJson<DictItemUpdate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:edit")?;
    check_label(&body.label)?;
    check_value(&body.value)?;
    check_len("remark", &body.remark, 255)?;

    let mut it = DictItem::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    it.label = body.label.trim().to_string();
    it.value = body.value.trim().to_string();
    it.sort = body.sort;
    it.status = body.status;
    it.remark = body.remark;
    it.updated_at = now();
    match state.db.update(&it).await {
        Ok(_) => {}
        Err(OrmError::DuplicateKey(_)) => {
            return Err(ApiError::BadRequest("该类型下字典值已存在".into()));
        }
        Err(e) => return Err(e.into()),
    }
    Ok(ok(Value::Null))
}

pub async fn item_remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<u64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:dict:remove")?;
    let n = state.db.delete::<DictItem>(id).await.map_err(ApiError::from)?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(ok(Value::Null))
}

/// 取一批导出数据（keyset：`id < last`，首页不带该条件）。
async fn export_batch(
    state: &AppState,
    base: &bee_orm::QuerySet<DictItem>,
    last: Option<u64>,
) -> Result<Vec<(u64, String)>, ApiError> {
    let mut qs = base.clone().order_by("id DESC").limit(csv::BATCH);
    if let Some(last) = last {
        qs = qs.filter_raw("id < ?", &[last]);
    }
    let rows = qs.fetch_all(&state.db).await.map_err(ApiError::from)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.id,
                csv::row(&[
                    r.id.to_string(),
                    r.type_code,
                    r.label,
                    r.value,
                    r.sort.to_string(),
                    if r.status == 1 { "启用".into() } else { "停用".into() },
                    r.remark,
                    r.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                ]),
            )
        })
        .collect())
}

/// 导出当前筛选结果（不含分页）：筛选条件与列表共用 `filtered_items()`，导出范围不会漂移。
pub async fn item_export(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<DictItemQuery>,
) -> Result<Response, ApiError> {
    auth.require("system:dict:list")?;
    let base = filtered_items(&q)?;
    csv::streamed(
        format!("dict-items-{}.csv", csv::today()),
        header_cells(),
        move |last| {
            let state = state.clone();
            let base = base.clone();
            async move { export_batch(&state, &base, last).await }
        },
    )
    .await
}

fn header_cells() -> Vec<String> {
    ["ID", "类型编码", "标签", "值", "排序", "状态", "备注", "创建时间"]
        .map(String::from)
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_accepts_only_lowercase_ident_chars() {
        assert!(valid_code("user_sex"));
        assert!(valid_code("a1"));
        assert!(valid_code(&"x".repeat(64)));
        // 长度
        assert!(!valid_code("a"), "1 位太短");
        assert!(!valid_code(&"x".repeat(65)), "65 位太长");
        // 字符集
        assert!(!valid_code("UserSex"), "大写不接受（不静默改写）");
        assert!(!valid_code("user-sex"));
        assert!(!valid_code("用户性别"));
        assert!(!valid_code("a b"));
        assert!(!valid_code(""));
    }

    #[test]
    fn empty_name_value_rejected() {
        assert!(check_name("  ").is_err());
        assert!(check_name("性别").is_ok());
        assert!(check_value(" ").is_err());
        assert!(check_value("1").is_ok());
    }
}
