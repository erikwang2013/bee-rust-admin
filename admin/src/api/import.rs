// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! CSV 导入（扩展工程项 3）：管理员 / 部门 / 字典项三种。
//!
//! 契约：`GET /api/v1/imports/template?kind=…` 下模板（只有表头），
//! `POST /api/v1/imports?kind=…`（multipart 字段 `file`）导入。
//!
//! 口径（施工图「全有或全无」）：
//! 1. **先全量校验**，逐行收集报告（行号 + 原因，好行也回显 `ok:true`）；
//! 2. 只要有一行坏，**零写入**并回 `created:0` + 报告（HTTP 仍是 200 —— 请求本身被
//!    正常处理了，坏在文件内容，前端按 `created`/`rows` 渲染报告表）；
//! 3. 全对才在**单事务**里落库，中途任何一步失败整批回滚。
//!
//! 两条「不另起一套」：权限复用各模块的 `:add` 码（导入是新增数据的另一种入口），
//! 校验复用同步接口那批函数（口令策略 / 名称长度 / 数据范围闸门）——
//! 导入的参数若比同步接口宽，就是个绕过闸门的后门。
//!
//! shortcut: 只认 UTF-8（Excel 默认另存是 GBK，会被拒并提示另存为 CSV UTF-8）；
//! 等真实诉求来了再接 encoding_rs 做编码嗅探。行数上限 5000（事务规模 + 响应体都框住）。

use crate::api::admin::{sealed, validate_profile, validate_username};
use crate::api::csv::{BOM, headers as csv_headers, row as csv_row};
use crate::api::dept::{DeptBody, validate_body};
use crate::api::dict::{check_label, check_value};
use crate::api::{check_len, dedup_ids};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppMultipart, ok};
use crate::models::{Admin, Dept, DictItem, DictType, Role};
use crate::state::AppState;
use crate::util::{hash_password, now};
use axum::Json;
use axum::extract::{Multipart, Query, State};
use axum::response::{IntoResponse, Response};
use bee_orm::pool::mysql::CheckedConn;
use bee_orm::{Model, Value};
use serde::Deserialize;
use serde_json::{Value as JsonVal, json};
use std::collections::HashSet;

/// 一次最多导入的数据行数。
const MAX_ROWS: usize = 5000;

#[derive(Deserialize)]
pub struct KindQuery {
    /// `admin` / `dept` / `dict-item`。
    pub kind: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Admin,
    Dept,
    DictItem,
}

impl Kind {
    fn parse(s: &str) -> Result<Self, ApiError> {
        match s {
            "admin" => Ok(Kind::Admin),
            "dept" => Ok(Kind::Dept),
            "dict-item" => Ok(Kind::DictItem),
            other => Err(ApiError::BadRequest(format!(
                "不认识的导入类型 {other:?}（可选 admin / dept / dict-item）"
            ))),
        }
    }

    /// 回给前端的种类名（与 query 参数同形），也是模板文件名的前缀。
    fn name(self) -> &'static str {
        match self {
            Kind::Admin => "admin",
            Kind::Dept => "dept",
            Kind::DictItem => "dict-item",
        }
    }

    /// 复用各模块的新增权限码：能新增就能导入（不新增权限码，批次 B 台账有 Ruling）。
    fn perm(self) -> &'static str {
        match self {
            Kind::Admin => "system:admin:add",
            Kind::Dept => "system:dept:add",
            Kind::DictItem => "system:dict:add",
        }
    }

    /// 表头 = 解析时逐字比对的列名（模板、前端提示、usage 文档都按这份走）。
    fn headers(self) -> &'static [&'static str] {
        match self {
            Kind::Admin => &["用户名", "密码", "昵称", "邮箱", "手机号", "部门", "角色", "状态", "备注"],
            Kind::Dept => &["部门名称", "上级部门", "负责人", "联系电话", "排序", "状态"],
            Kind::DictItem => &["类型编码", "标签", "值", "排序", "状态", "备注"],
        }
    }
}

/// 校验通过、待落库的一行。`role_ids` 只有管理员行有（关联表另插）。
enum Item {
    Admin { row: Admin, role_ids: Vec<i64> },
    Dept(Dept),
    Dict(DictItem),
}

/// 逐行报告：`msg` 只在坏行非空。
struct Checked {
    line: u64,
    item: Option<Item>,
    msg: String,
}

#[apidoc::title("下载导入模板")]
#[apidoc::desc("只回表头（CSV + BOM，Excel 直接可编辑）。列名不能改：导入时逐字比对表头，改过会整份拒绝。数据行请照模板列序填，空行会被忽略。")]
#[apidoc::url("/api/v1/imports/template")]
#[apidoc::method("GET")]
#[apidoc::tag("数据导入")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "kind", ty = "string", required, desc = "导入类型：admin 管理员 / dept 部门 / dict-item 字典项")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("403")]
#[apidoc::returned(name = "data", ty = "file", desc = "CSV 附件（text/csv; charset=utf-8）")]
pub async fn template(
    auth: Auth,
    Query(q): Query<KindQuery>,
) -> Result<Response, ApiError> {
    let kind = Kind::parse(&q.kind)?;
    auth.require(kind.perm())?;
    let cells: Vec<String> = kind.headers().iter().map(|h| h.to_string()).collect();
    let body = format!("{}{}", BOM, csv_row(&cells));
    let name = format!("{}_import_template.csv", kind.name());
    Ok((csv_headers(&name), body).into_response())
}

#[apidoc::title("CSV 导入")]
#[apidoc::desc("multipart 表单（字段名 file）。先全量校验再落库：有一行不合规就整批不写（created=0），逐行报告给行号与原因，好行也是 ok=true。管理员行的「部门」「角色」按名称定位（角色多个用 | 分隔，与导出同分隔符）；部门行的「上级部门」必须是库里已有的部门。")]
#[apidoc::url("/api/v1/imports")]
#[apidoc::method("POST")]
#[apidoc::tag("数据导入")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "kind", ty = "string", required, desc = "导入类型：admin 管理员 / dept 部门 / dict-item 字典项")]
#[apidoc::param(name = "file", ty = "file", required, desc = "CSV 文件（UTF-8 / CSV UTF-8，带不带 BOM 都认）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("403")]
#[apidoc::returned(name = "data", ty = "object", desc = "导入报告", children = [
        {name = "kind", ty = "string", desc = "导入类型"},
        {name = "total", ty = "int", desc = "数据行数（不含表头与空行）"},
        {name = "created", ty = "int", desc = "实际写入条数：全对 = total，有坏行 = 0"},
        {name = "rows", ty = "array", desc = "逐行报告", children = [
            {name = "line", ty = "int", desc = "文件行号（表头是第 1 行）"},
            {name = "ok", ty = "bool", desc = "该行是否通过校验"},
            {name = "msg", ty = "string", desc = "不通过的原因（通过时为空串）"},
        ]},
    ])]
pub async fn import(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<KindQuery>,
    mut multipart: AppMultipart,
) -> Result<Json<JsonVal>, ApiError> {
    let kind = Kind::parse(&q.kind)?;
    auth.require(kind.perm())?;
    let bytes = read_file(&mut multipart.0).await?;
    let text = decode(&bytes)?;

    let checked = check_all(&state, &auth, kind, &text).await?;
    let total = checked.len();
    let created = if checked.iter().any(|c| c.item.is_none()) {
        0 // 有坏行：一条都不写（报告里已逐行说明）
    } else {
        write_all(&state, checked.iter().filter_map(|c| c.item.as_ref())).await?
    };
    let rows: Vec<JsonVal> = checked
        .iter()
        .map(|c| json!({ "line": c.line, "ok": c.item.is_some(), "msg": c.msg }))
        .collect();
    Ok(ok(json!({ "kind": kind.name(), "total": total, "created": created, "rows": rows })))
}

/// 取表单里的 `file` 字段（字段名与前端逐字一致）。
async fn read_file(mp: &mut Multipart) -> Result<Vec<u8>, ApiError> {
    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("上传表单解析失败: {e}")))?
    {
        if field.name() == Some("file") {
            return field
                .bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| ApiError::BadRequest(format!("读取上传文件失败: {e}")));
        }
    }
    Err(ApiError::BadRequest("表单里没有 file 字段".into()))
}

/// 字节 → 文本：去 BOM + 认 UTF-8。
fn decode(bytes: &[u8]) -> Result<String, ApiError> {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(body.to_vec())
        .map_err(|_| ApiError::BadRequest("文件不是 UTF-8 编码（Excel 请用「CSV UTF-8」另存）".into()))
}

/// 校验全表：表头 → 预载参照数据 → 逐行。返回逐行报告（好行带待落库实体）。
async fn check_all(
    state: &AppState,
    auth: &Auth,
    kind: Kind,
    text: &str,
) -> Result<Vec<Checked>, ApiError> {
    // flexible：列数不对的行也读得出来，好在报告里点名（而不是整份文件解析失败）
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(text.as_bytes());
    let head = rdr
        .headers()
        .map_err(|e| ApiError::BadRequest(format!("解析表头失败: {e}")))?
        .clone();
    let want = kind.headers();
    let same = head.len() == want.len()
        && want.iter().zip(head.iter()).all(|(w, got)| w.trim() == got.trim());
    if !same {
        return Err(ApiError::BadRequest(format!(
            "表头与模板不一致，应为: {}",
            want.join(",")
        )));
    }

    let mut ctx = Ctx::load(state, kind).await?;
    let mut out: Vec<Checked> = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| ApiError::BadRequest(format!("CSV 解析失败: {e}")))?;
        if rec.iter().all(|c| c.trim().is_empty()) {
            continue; // 空行（含末尾换行产生的幻影行）
        }
        if out.len() >= MAX_ROWS {
            return Err(ApiError::BadRequest(format!("一次最多导入 {MAX_ROWS} 行")));
        }
        let line = rec.position().map(|p| p.line()).unwrap_or(0);
        let r = match kind {
            Kind::Admin => admin_item(state, auth, &mut ctx, &rec).await,
            Kind::Dept => dept_item(state, &mut ctx, &rec),
            Kind::DictItem => dict_item(state, &mut ctx, &rec),
        };
        match r {
            Ok(item) => out.push(Checked { line, item: Some(item), msg: String::new() }),
            // 内部错误不是这一行的问题：整份冒出去（别伪装成「你的文件写错了」）
            Err(e @ ApiError::Internal(_)) => return Err(e),
            Err(e) => out.push(Checked { line, item: None, msg: reason(&e) }),
        }
    }
    Ok(out)
}

/// 行内报告用的原因文本：取错误的中文 `msg`（`Internal` 的细节只留日志，不外泄）。
fn reason(e: &ApiError) -> String {
    match e {
        ApiError::BadRequest(m) | ApiError::Forbidden(m) | ApiError::NotFoundMsg(m) => m.clone(),
        ApiError::Coded { msg, .. } => msg.clone(),
        ApiError::Unauthorized => "登录已失效，请重新登录".into(),
        ApiError::NotFound => "资源不存在".into(),
        ApiError::Internal(_) => "服务器内部错误".into(),
    }
}

/// 一次预载的参照数据 + 文件内去重集合（行内校验不再查库；5000 行 = 4 次查询）。
struct Ctx {
    depts: Vec<Dept>,
    roles: Vec<Role>,
    /// 库里已有的用户名。
    usernames: HashSet<String>,
    /// 本文件里已出现过的用户名（同文件重复也算「已存在」）。
    seen_users: HashSet<String>,
    dict_codes: HashSet<String>,
    dict_pairs: HashSet<(String, String)>,
    seen_pairs: HashSet<(String, String)>,
}

impl Ctx {
    async fn load(state: &AppState, kind: Kind) -> Result<Self, ApiError> {
        let mut ctx = Ctx {
            depts: Vec::new(),
            roles: Vec::new(),
            usernames: HashSet::new(),
            seen_users: HashSet::new(),
            dict_codes: HashSet::new(),
            dict_pairs: HashSet::new(),
            seen_pairs: HashSet::new(),
        };
        if matches!(kind, Kind::Admin | Kind::Dept) {
            ctx.depts = Dept::query().all(&state.db).await.map_err(ApiError::from)?;
        }
        if kind == Kind::Admin {
            ctx.roles = Role::query().all(&state.db).await.map_err(ApiError::from)?;
            // 只取一列：admin 表可能有几万行，用户名做唯一性预检就够了
            ctx.usernames = state
                .db
                .query("SELECT username FROM admin", &[])
                .await
                .map_err(ApiError::from)?
                .iter()
                .filter_map(|r| r.get("username").and_then(|v| v.as_str()).map(str::to_string))
                .collect();
        }
        if kind == Kind::DictItem {
            ctx.dict_codes = DictType::query()
                .all(&state.db)
                .await
                .map_err(ApiError::from)?
                .into_iter()
                .map(|t| t.code)
                .collect();
            ctx.dict_pairs = state
                .db
                .query("SELECT type_code, value FROM dict_item", &[])
                .await
                .map_err(ApiError::from)?
                .iter()
                .filter_map(|r| {
                    Some((
                        r.get("type_code")?.as_str()?.to_string(),
                        r.get("value")?.as_str()?.to_string(),
                    ))
                })
                .collect();
        }
        Ok(ctx)
    }
}

/// 单元格（列数不足时当空串：报告里会点名，不用 unwrap 炸掉）。
fn cell(rec: &csv::StringRecord, i: usize) -> &str {
    rec.get(i).unwrap_or("")
}

fn parse_int(s: &str, label: &str) -> Result<i32, ApiError> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(0);
    }
    t.parse()
        .map_err(|_| ApiError::BadRequest(format!("{label}必须是整数，收到 {t:?}")))
}

/// 状态列：空 / 1 / 启用 → 1，0 / 禁用 → 0（与页面上那套中文下拉对齐）。
fn parse_status(s: &str) -> Result<i8, ApiError> {
    match s.trim() {
        "" | "1" | "启用" => Ok(1),
        "0" | "禁用" => Ok(0),
        other => Err(ApiError::BadRequest(format!(
            "状态只认 1/0 或「启用」「禁用」，收到 {other:?}"
        ))),
    }
}

/// 按名称定位部门（空 = 根 / 无部门）。重名定位不了就直接报错，不随便挑一个。
fn pick_dept(depts: &[Dept], name: &str, field: &str) -> Result<i64, ApiError> {
    let n = name.trim();
    if n.is_empty() {
        return Ok(0);
    }
    match depts.iter().filter(|d| d.name == n).count() {
        1 => Ok(depts.iter().find(|d| d.name == n).map(|d| d.id).unwrap_or(0)),
        0 => Err(ApiError::BadRequest(format!("{field}「{n}」不存在"))),
        _ => Err(ApiError::BadRequest(format!("{field}「{n}」不唯一，请先把重名的部门改名"))),
    }
}

/// 角色列：名称用 `|` 分隔（与导出的数组列同分隔符）。空 = 不挂角色。
fn pick_roles(roles: &[Role], names: &str) -> Result<Vec<i64>, ApiError> {
    let mut ids = Vec::new();
    for name in names.split('|').map(str::trim).filter(|s| !s.is_empty()) {
        match roles.iter().filter(|r| r.name == name).count() {
            1 => ids.extend(roles.iter().find(|r| r.name == name).map(|r| r.id)),
            0 => return Err(ApiError::BadRequest(format!("角色「{name}」不存在"))),
            _ => return Err(ApiError::BadRequest(format!("角色「{name}」不唯一，请先改名"))),
        }
    }
    Ok(dedup_ids(ids))
}

/// 管理员行。id 在校验期就发号（雪花号有间隔无所谓），写完不用的号作废。
async fn admin_item(
    state: &AppState,
    auth: &Auth,
    ctx: &mut Ctx,
    rec: &csv::StringRecord,
) -> Result<Item, ApiError> {
    let username = cell(rec, 0).trim().to_string();
    let password = cell(rec, 1);
    let nickname = cell(rec, 2).trim().to_string();
    let email = cell(rec, 3).trim().to_string();
    let phone = cell(rec, 4).trim().to_string();
    let remark = cell(rec, 8).trim().to_string();
    validate_username(&username)?;
    crate::api::admin::validate_password(password, "密码")?;
    validate_profile(&nickname, &email, &phone, &remark)?;
    if ctx.usernames.contains(&username) || !ctx.seen_users.insert(username.clone()) {
        return Err(ApiError::BadRequest("用户名已存在".into()));
    }
    let dept_id = pick_dept(&ctx.depts, cell(rec, 5), "部门")?;
    let role_ids = pick_roles(&ctx.roles, cell(rec, 6))?;
    // 数据范围闸门与同步新建**同一份**函数：导入不能是绕过 B5 的后门
    datascope::ensure_dept_in_scope(auth, &state.db, dept_id).await?;
    datascope::ensure_roles_grantable(auth, &state.db, &role_ids).await?;
    let status = parse_status(cell(rec, 7))?;

    let row = Admin {
        id: state.next_id()?,
        username,
        password: hash_password(password),
        nickname,
        email: sealed(&state.crypto, &email)?,
        phone: sealed(&state.crypto, &phone)?,
        sex: 0,
        avatar: String::new(),
        dept_id,
        status,
        is_super: 0,
        totp_secret: String::new(),
        totp_enabled: 0,
        token_version: 0,
        last_login_at: None,
        last_login_ip: String::new(),
        remark,
        created_at: now(),
        updated_at: now(),
    };
    Ok(Item::Admin { row, role_ids })
}

/// 部门行：上级按名称定位，**必须是库里已有的部门**（文件里新建的当不了上级 ——
/// 否则要么一遍扫描里排顺序，要么多趟解析，都不值得）。
fn dept_item(state: &AppState, ctx: &mut Ctx, rec: &csv::StringRecord) -> Result<Item, ApiError> {
    let name = cell(rec, 0).trim().to_string();
    let leader = cell(rec, 2).trim().to_string();
    let phone = cell(rec, 3).trim().to_string();
    let sort = parse_int(cell(rec, 4), "排序")?;
    let status = parse_status(cell(rec, 5))?;
    let parent_id = pick_dept(&ctx.depts, cell(rec, 1), "上级部门")?;
    validate_body(&DeptBody {
        parent_id,
        name: name.clone(),
        sort,
        leader: leader.clone(),
        phone: phone.clone(),
        status,
    })?;
    Ok(Item::Dept(Dept {
        id: state.next_id()?,
        parent_id,
        name,
        sort,
        leader,
        phone,
        status,
        created_at: now(),
        updated_at: now(),
    }))
}

/// 字典项行：类型编码必须已存在；`(类型, 值)` 是复合唯一键。
fn dict_item(state: &AppState, ctx: &mut Ctx, rec: &csv::StringRecord) -> Result<Item, ApiError> {
    let type_code = cell(rec, 0).trim().to_string();
    let label = cell(rec, 1).trim().to_string();
    let value = cell(rec, 2).trim().to_string();
    let sort = parse_int(cell(rec, 3), "排序")?;
    let status = parse_status(cell(rec, 4))?;
    let remark = cell(rec, 5).trim().to_string();
    check_label(&label)?;
    check_value(&value)?;
    check_len("remark", &remark, 255)?;
    if !ctx.dict_codes.contains(&type_code) {
        return Err(ApiError::BadRequest("字典类型不存在".into()));
    }
    let key = (type_code.clone(), value.clone());
    if ctx.dict_pairs.contains(&key) || !ctx.seen_pairs.insert(key) {
        return Err(ApiError::BadRequest("该类型下字典值已存在".into()));
    }
    Ok(Item::Dict(DictItem {
        id: state.next_id()?,
        type_code,
        label,
        value,
        sort,
        status,
        remark,
        created_at: now(),
        updated_at: now(),
    }))
}

/// 单事务落库（走到这里说明全表都过了校验）。中途任何一步失败 → 回滚，一条不留。
async fn write_all<'a>(
    state: &AppState,
    items: impl Iterator<Item = &'a Item>,
) -> Result<i64, ApiError> {
    let mut conn = state.db.get().await.map_err(ApiError::from)?;
    conn.begin().await.map_err(ApiError::from)?;
    let mut created = 0i64;
    for item in items {
        let r = match item {
            Item::Admin { row, role_ids } => {
                insert_tx(&mut conn, row).await.map_err(|e| conflict(e, "用户名已存在"))?;
                for rid in role_ids {
                    conn.execute(
                        "INSERT INTO admin_role (admin_id, role_id) VALUES (?, ?)",
                        &[Value::from(row.id), Value::from(*rid)],
                    )
                    .await
                    .map_err(ApiError::from)?;
                }
                Ok(())
            }
            Item::Dept(d) => insert_tx(&mut conn, d).await,
            Item::Dict(d) => insert_tx(&mut conn, d).await.map_err(|e| conflict(e, "该类型下字典值已存在")),
        };
        if let Err(e) = r {
            // 回滚失败只记日志：原始错误更有用（回滚失败本身会让连接弃用）
            if let Err(re) = conn.rollback().await {
                tracing::error!("导入回滚失败: {re:?}");
            }
            return Err(e);
        }
        created += 1;
    }
    conn.commit().await.map_err(ApiError::from)?;
    Ok(created)
}

/// 唯一键冲突 → 该导入的专属文案：预检之后又被别人占了的窄窗口。
fn conflict(e: ApiError, taken: &str) -> ApiError {
    match e {
        ApiError::BadRequest(_) => ApiError::BadRequest(taken.into()),
        other => other,
    }
}

/// 事务内单行插入。`CheckedConn` 不是 `Db`（见 bee_orm），`Model::insert` 用不了 ——
/// 这里照它的拼法来一份：列名来自模型的 `insert_values`，值一律走绑定参数。
/// 导入的模型都不声明 `auto_now`；哪天有模型声明了，宁可直接报错也不要写出没有时间戳的行。
async fn insert_tx<M: Model>(conn: &mut CheckedConn, row: &M) -> Result<(), ApiError> {
    if !M::auto_now_add_columns().is_empty() || !M::auto_now_columns().is_empty() {
        return Err(ApiError::internal(format!(
            "{} 声明了 auto_now，导入路径不支持（请在 insert_tx 里补时间戳注入）",
            M::table_name()
        )));
    }
    let values = row.insert_values();
    if values.is_empty() {
        return Err(ApiError::internal(format!("{} 没有任何列可写", M::table_name())));
    }
    let columns: Vec<&str> = values.iter().map(|(c, _)| *c).collect();
    let placeholders = vec!["?"; values.len()].join(", ");
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({placeholders})",
        M::table_name(),
        columns.join(", ")
    );
    let params: Vec<Value> = values.into_iter().map(|(_, v)| v).collect();
    conn.execute(&sql, &params).await.map_err(ApiError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dept(id: i64, name: &str) -> Dept {
        Dept {
            id,
            parent_id: 0,
            name: name.into(),
            sort: 0,
            leader: String::new(),
            phone: String::new(),
            status: 1,
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn kind_parses_and_rejects_unknown() {
        assert!(Kind::parse("admin").is_ok());
        assert!(Kind::parse("dept").is_ok());
        assert!(Kind::parse("dict-item").is_ok());
        assert!(Kind::parse("dict").is_err(), "字典项的种类名是 dict-item");
        assert!(Kind::parse("").is_err());
        // 三种模板列数与解析列序一一对应（表头即契约）
        assert_eq!(Kind::Admin.headers().len(), 9);
        assert_eq!(Kind::Dept.headers().len(), 6);
        assert_eq!(Kind::DictItem.headers().len(), 6);
    }

    /// 名称定位：空 = 根；唯一命中 = 该 id；不存在 / 重名都是错误（不猜）。
    #[test]
    fn pick_dept_resolves_by_unique_name() {
        let all = vec![dept(1, "总部"), dept(2, "研发部"), dept(3, "研发部")];
        assert_eq!(pick_dept(&all, "", "部门").unwrap(), 0);
        assert_eq!(pick_dept(&all, " 总部 ", "部门").unwrap(), 1);
        let e = pick_dept(&all, "不存在", "上级部门").unwrap_err();
        assert!(reason(&e).contains("上级部门「不存在」不存在"), "{:?}", reason(&e));
        let e = pick_dept(&all, "研发部", "部门").unwrap_err();
        assert!(reason(&e).contains("不唯一"), "{:?}", reason(&e));
    }

    #[test]
    fn pick_roles_splits_and_dedups() {
        let roles: Vec<Role> = ["甲", "乙"]
            .iter()
            .enumerate()
            .map(|(i, n)| Role {
                id: i as i64 + 1,
                name: (*n).into(),
                code: format!("r{i}"),
                sort: 0,
                data_scope: 1,
                status: 1,
                remark: String::new(),
                created_at: now(),
                updated_at: now(),
            })
            .collect();
        assert_eq!(pick_roles(&roles, "").unwrap(), Vec::<i64>::new());
        // 与关联写入同一口径（`dedup_ids` 会排序去重）：重复的角色名不会插两次
        assert_eq!(pick_roles(&roles, " 乙 | 甲 |乙| ").unwrap(), vec![1, 2]);
        let e = pick_roles(&roles, "丙").unwrap_err();
        assert!(reason(&e).contains("角色「丙」不存在"), "{:?}", reason(&e));
    }

    #[test]
    fn parse_status_accepts_both_dialects() {
        for (s, want) in [("", 1), ("1", 1), ("启用", 1), (" 启用 ", 1), ("0", 0), ("禁用", 0)] {
            assert_eq!(parse_status(s).unwrap(), want, "{s:?}");
        }
        assert!(parse_status("2").is_err());
        assert!(parse_status("yes").is_err());
    }

    #[test]
    fn parse_int_defaults_to_zero_but_rejects_junk() {
        assert_eq!(parse_int("", "排序").unwrap(), 0);
        assert_eq!(parse_int(" 12 ", "排序").unwrap(), 12);
        assert_eq!(parse_int("-3", "排序").unwrap(), -3);
        assert!(parse_int("三", "排序").is_err());
    }

    #[test]
    fn decode_strips_bom_and_rejects_non_utf8() {
        assert_eq!(decode(b"a,b").unwrap(), "a,b");
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("中文".as_bytes());
        assert_eq!(decode(&bom).unwrap(), "中文");
        // GBK 的「中」（0xD6 0xD0）不是合法 UTF-8：必须拒，且提示怎么救
        let e = decode(&[0xD6, 0xD0]).unwrap_err();
        assert!(reason(&e).contains("CSV UTF-8"), "{:?}", reason(&e));
    }
}
