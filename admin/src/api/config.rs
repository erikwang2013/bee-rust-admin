// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 系统配置（扩展工程项 1）：分组键值对。
//! 除 `GET /configs/public`（免登录，只回 is_public=1 的键）外都要权限码：
//! 读 `system:config:list`，写（增/改/删）`system:config:edit`。
use crate::api::check_len;
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::hid;
use crate::models::SysConfig;
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use bee_orm::{Model, OrmError};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// 配置键：小写字母/数字/下划线，点号分段的命名空间（`site.copyright`），2~128 位。
/// 段不许为空（`a..b` / 首尾点都拒）；键是公开读的 JSON 字段名，也图它好打字。
fn valid_key(key: &str) -> bool {
    let n = key.chars().count();
    (2..=128).contains(&n)
        && key.split('.').all(|seg| {
            !seg.is_empty()
                && seg.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// 分组名：小写字母/数字/下划线，最长 64；空串 = 未分组（合法）。
fn valid_group(group: &str) -> bool {
    group.chars().count() <= 64
        && group.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// 值的三种类型：单行 / 多行 / 布尔。布尔落库也存 `"true"` / `"false"` 字符串，
/// 公开读时再转 JSON 布尔 —— 库里值的形态只有一种，不用猜。
const TYPES: [&str; 3] = ["string", "text", "bool"];

/// 敏感键（值以密文形态存在、由专用页面维护）：本页**不列、不改、不删**。
/// 列出来等于把口令键暴露给每个有 `system:config:list` 的人，改一下就能把
/// 「落库加密」换成明文 —— `加密存` 就成了摆设。名单是显式的：加新键时记得往这儿补。
const SECRET_KEYS: [&str; 1] = [crate::notify::KEY_SMTP_PASSWORD];

fn is_secret_key(key: &str) -> bool {
    SECRET_KEYS.contains(&key)
}

/// 命中敏感键的统一拒绝文案（改/删共用）。
fn secret_key_rejected() -> ApiError {
    ApiError::BadRequest("该配置项由「通知设置」页维护，不能在这里修改".into())
}

/// 公开读的值转换：`bool` → JSON 布尔（`"1"` 也认，大小写不敏感、先 trim —— 库里被
/// 手写成 `"TRUE"` / `" true "` 时不至于被静默取反）；其余原样字符串。
fn public_value(value_type: &str, value: &str) -> Value {
    match value_type {
        "bool" => {
            let v = value.trim();
            json!(v == "1" || v.eq_ignore_ascii_case("true"))
        }
        _ => json!(value),
    }
}

/// 键重复 → 本接口的专属文案（唯一键冲突的识别同 `dict::code_taken`）。
fn key_taken(e: OrmError) -> ApiError {
    match ApiError::from(e) {
        ApiError::BadRequest(_) => ApiError::BadRequest("配置键已存在".into()),
        other => other,
    }
}

#[derive(Deserialize)]
pub struct ConfigQuery {
    /// 分组过滤（空 = 全部）。配置表按设计是几十条量级，不分页。
    pub group: Option<String>,
}

#[derive(Deserialize)]
pub struct ConfigCreate {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default = "default_type")]
    pub value_type: String,
    #[serde(default)]
    pub group: String,
    /// 默认 **0（私有）**：新建的键绝不默认外泄，要公开得显式开。
    #[serde(default)]
    pub is_public: i8,
    #[serde(default)]
    pub remark: String,
}

fn default_type() -> String {
    "string".into()
}

/// 部分更新：只改传了的字段。全量替换会把前端没带的 `value_type` 静默归位 `string`，
/// 一个布尔开关就此变成输入框 —— 太危险，所以这里全是 `Option`。
#[derive(Deserialize)]
pub struct ConfigUpdate {
    pub value: Option<String>,
    pub value_type: Option<String>,
    pub is_public: Option<i8>,
    pub remark: Option<String>,
}

fn check_value(value: &str) -> Result<(), ApiError> {
    check_len("value", value, 2000)
}

fn check_type(t: &str) -> Result<(), ApiError> {
    if TYPES.contains(&t) {
        return Ok(());
    }
    Err(ApiError::BadRequest(format!("配置类型只能是 {}", TYPES.join(" / "))))
}

#[apidoc::title("系统配置列表")]
#[apidoc::desc("全部配置（按 key 字典序）；配置表按设计是几十条量级，不分页。可选 group 过滤")]
#[apidoc::url("/api/v1/configs")]
#[apidoc::method("GET")]
#[apidoc::tag("系统配置")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "group", ty = "string", desc = "分组过滤（空 = 全部）")]
#[apidoc::response_status("200")]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<ConfigQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:config:list")?;
    let mut qs = SysConfig::query();
    if let Some(g) = q.group.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_eq("group_name", g.trim()).map_err(ApiError::from)?;
    }
    let mut rows = qs.order_by("config_key ASC").all(&state.db).await.map_err(ApiError::from)?;
    rows.retain(|c| !is_secret_key(&c.key)); // 口令键不上这一页（见 SECRET_KEYS）
    let total = rows.len();
    Ok(ok(json!({ "list": rows, "total": total })))
}

/// 公开读：免登录（登录页/侧边栏页脚要用）。只回 is_public=1 的键，
/// 值按类型转换（bool → JSON 布尔）。**没有缓存**：改配置后刷新立即可见。
#[apidoc::title("公开配置（免登录）")]
#[apidoc::desc("登录页/页脚读取用：只回标记为公开的键，值按类型转换（bool → JSON 布尔）。不带 token")]
#[apidoc::url("/api/v1/configs/public")]
#[apidoc::method("GET")]
#[apidoc::tag("系统配置")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "{ \"site.copyright\": \"…\", \"site.footer_enabled\": true, … }")]
pub async fn public_list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows = SysConfig::query()
        .filter_eq("is_public", 1)
        .map_err(ApiError::from)?
        .order_by("config_key ASC")
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;
    let mut map = Map::new();
    for c in rows {
        map.insert(c.key, public_value(&c.value_type, &c.value));
    }
    Ok(ok(Value::Object(map)))
}

#[apidoc::title("新建系统配置")]
#[apidoc::desc("key 唯一且小写字母/数字/下划线（点号分段）；is_public 默认 0（私有，要公开得显式开）")]
#[apidoc::url("/api/v1/configs")]
#[apidoc::method("POST")]
#[apidoc::tag("系统配置")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "key", ty = "string", required, desc = "配置键（唯一，如 site.copyright）")]
#[apidoc::param(name = "value", ty = "string", desc = "配置值（字符串落库）")]
#[apidoc::param(name = "value_type", ty = "string", desc = "string / text / bool，默认 string")]
#[apidoc::param(name = "group", ty = "string", desc = "分组（如 site），默认空 = 未分组")]
#[apidoc::param(name = "is_public", ty = "int", desc = "1 公开 / 0 私有，默认 0")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注（管理页的说明文字）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<ConfigCreate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:config:edit")?;
    let key = body.key.trim();
    if !valid_key(key) {
        return Err(ApiError::BadRequest(
            "配置键只能是小写字母、数字、下划线与点号分段，长度 2~128".into(),
        ));
    }
    check_value(&body.value)?;
    check_type(&body.value_type)?;
    if !valid_group(&body.group) {
        return Err(ApiError::BadRequest("分组名只能是小写字母、数字、下划线，最长 64".into()));
    }
    check_len("remark", &body.remark, 255)?;

    let c = SysConfig {
        id: state.next_id()?,
        key: key.to_string(),
        value: body.value,
        value_type: body.value_type,
        group: body.group,
        is_public: body.is_public,
        remark: body.remark,
        created_at: now(),
        updated_at: now(),
    };
    // id 已在上面发号，insert 就够（create 的读回是给自增主键用的）
    c.insert(&state.db).await.map_err(key_taken)?;
    Ok(ok(json!({ "id": hid::enc(c.id) })))
}

#[apidoc::title("编辑系统配置")]
#[apidoc::desc("部分更新：只改传了的字段。key / group 是定位与分组键，更新体里没有它们")]
#[apidoc::url("/api/v1/configs/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("系统配置")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "配置对外 id（hashids 短串）")]
#[apidoc::param(name = "value", ty = "string", desc = "配置值（不传不动）")]
#[apidoc::param(name = "value_type", ty = "string", desc = "string / text / bool（不传不动）")]
#[apidoc::param(name = "is_public", ty = "int", desc = "1 公开 / 0 私有（不传不动）")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注（不传不动）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("404")]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<ConfigUpdate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:config:edit")?;
    let id = hid::dec(&id)?;
    if let Some(v) = &body.value {
        check_value(v)?;
    }
    if let Some(t) = &body.value_type {
        check_type(t)?;
    }
    if let Some(r) = &body.remark {
        check_len("remark", r, 255)?;
    }

    let mut c = SysConfig::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if is_secret_key(&c.key) {
        return Err(secret_key_rejected());
    }

    if let Some(v) = body.value {
        c.value = v;
    }
    if let Some(t) = body.value_type {
        c.value_type = t;
    }
    if let Some(p) = body.is_public {
        c.is_public = p;
    }
    if let Some(r) = body.remark {
        c.remark = r;
    }
    c.updated_at = now();
    c.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("删除系统配置")]
#[apidoc::desc("不存在回 404。删公开键等于从登录页/页脚撤下（前端回落内置默认值）；内置键被删掉后，**下次启动服务时会被种子流程补回来**（补的是种子默认值，不是删前的值）")]
#[apidoc::url("/api/v1/configs/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("系统配置")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "配置对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:config:edit")?;
    let id = hid::dec(&id)?;
    // 先读出来判敏感键：删了它等于把口令丢了（下次启动种子会补一个空的回来）
    let key = SysConfig::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?
        .key;
    if is_secret_key(&key) {
        return Err(secret_key_rejected());
    }
    let n = SysConfig::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .delete(&state.db)
        .await
        .map_err(ApiError::from)?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(ok(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_accepts_dotted_namespace_only() {
        assert!(valid_key("site.copyright"));
        assert!(valid_key("a1"));
        assert!(valid_key(&"x".repeat(128)));
        assert!(!valid_key(&"x".repeat(129)), "超长");
        assert!(!valid_key("a"), "1 位太短");
        assert!(!valid_key("Site.Copyright"), "大写不接受（不静默改写）");
        assert!(!valid_key("site..copyright"), "空段");
        assert!(!valid_key(".site"), "首点");
        assert!(!valid_key("site."), "尾点");
        assert!(!valid_key("site copyright"));
        assert!(!valid_key("站点.版权"));
    }

    #[test]
    fn group_allows_empty_and_simple_names() {
        assert!(valid_group(""));
        assert!(valid_group("site"));
        assert!(valid_group("site_2"));
        assert!(!valid_group("Site"));
        assert!(!valid_group("a-b"));
        assert!(!valid_group(&"x".repeat(65)));
    }

    #[test]
    fn bool_values_cast_to_json_bool() {
        assert_eq!(public_value("bool", "true"), json!(true));
        assert_eq!(public_value("bool", "1"), json!(true));
        assert_eq!(public_value("bool", "TRUE"), json!(true), "大小写不敏感");
        assert_eq!(public_value("bool", " true "), json!(true), "先 trim（手写库值时常见）");
        assert_eq!(public_value("bool", "false"), json!(false));
        assert_eq!(public_value("bool", ""), json!(false), "空值按 false —— 绝不因解析失败回 true");
        assert_eq!(public_value("bool", "yes"), json!(false));
        // 其余类型原样字符串（含数字样式的值，不做隐式转换）
        assert_eq!(public_value("string", "true"), json!("true"));
        assert_eq!(public_value("text", "42"), json!("42"));
    }
}
