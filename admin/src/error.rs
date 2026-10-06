// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bee_orm::OrmError;
use serde_json::{Value, json};

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    Forbidden(String),
    NotFound,
    /// 404 + 自定义提示：默认的 `NotFound` 只说「资源不存在」，
    /// 有些场景要指明缺的是哪个东西（如「字典类型不存在」）
    NotFoundMsg(String),
    Internal(String),
    /// 带 `args` 的稳定错误码：`err` + `args` 进信封给前端拼文案，`msg` 仍是中文原文。
    /// 只有「参数是原始值」的码（too_long / throttled / not_registered / forbidden）用它，
    /// 静态消息走 `code_of` 表按 `msg` 自动挂码。
    Coded { status: StatusCode, err: &'static str, msg: String, args: Value },
}

impl ApiError {
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// 通用长度校验（`api::check_len`）：`field` 传**字段键**（如 `remark`），
    /// 不是中文标签——`args.field` 给前端查 `field.<键>` 文案表，`msg` 里拼中文。
    pub fn too_long(field: &str, max: usize) -> Self {
        Self::Coded {
            status: StatusCode::BAD_REQUEST,
            err: "common.too_long",
            msg: format!("{} 长度不能超过 {max} 个字符", field_label(field)),
            args: json!({ "field": field, "max": max }),
        }
    }

    /// 权限不足（`Auth::require` 失败的 403，用户最常撞见的一个）：
    /// `args.code` 是缺的那个权限码，前端可用它拼更具体的文案；`msg` 仍是中文原文。
    pub fn missing_permission(code: &str) -> Self {
        Self::Coded {
            status: StatusCode::FORBIDDEN,
            err: "auth.forbidden",
            msg: format!("缺少权限：{code}"),
            args: json!({ "code": code }),
        }
    }

    /// 登录限流（账号 / IP 维度失败次数超阈值或封禁中）。
    pub fn throttled(minutes: i64) -> Self {
        Self::Coded {
            status: StatusCode::TOO_MANY_REQUESTS,
            err: "auth.throttled",
            msg: format!("尝试过于频繁，请 {minutes} 分钟后再试"),
            args: json!({ "minutes": minutes }),
        }
    }

    /// 手动触发库里残留的旧任务 code（已从代码注册表下线）。
    pub fn job_not_registered(code: &str) -> Self {
        Self::Coded {
            status: StatusCode::CONFLICT,
            err: "job.not_registered",
            msg: format!("任务 {code} 未在代码中注册，不可手动触发"),
            args: json!({ "code": code }),
        }
    }
}

/// 稳定业务错误码表：`(中文原文 msg, err)`。
/// **这张表是前后端唯一耦合点**，行与 `err` 逐字照抄
/// `docs/superpowers/plans/2026-10-06-brd-v1.5-c-modules.md` 的「C3 契约 · 错误码表」。
/// 改某个 `msg` 文案前先看表：对不上的话 `err` 会静默消失（前端回落到中文 `msg`，
/// 不报错但译不了）。表里没有的消息（框架层提取器拒绝、`缺少权限：x` 等动态文案）
/// 按契约不带 `err`。带 `args` 的四条（too_long / throttled / not_registered / forbidden）
/// 在构造点显式挂码，不走这里。
const ERR_TABLE: &[(&str, &str)] = &[
    // auth
    ("用户名和密码不能为空", "auth.empty_credentials"),
    ("用户名或密码错误", "auth.bad_credentials"),
    ("账号已被禁用", "auth.disabled"),
    ("原密码错误", "auth.bad_old_password"),
    ("新密码至少 6 位", "auth.weak_new_password"),
    // avatar
    ("头像不能超过 512 KB", "avatar.too_large"),
    ("头像仅支持 PNG/JPEG", "avatar.bad_type"),
    ("头像数据格式错误", "avatar.bad_data"),
    ("头像 base64 解码失败", "avatar.bad_base64"),
    ("文件内容不是有效的图片", "avatar.not_an_image"),
    // admin
    ("密码至少 6 位", "admin.weak_password"),
    ("用户名长度需 3-64 个字符", "admin.username_length"),
    ("用户名只能包含字母、数字、下划线", "admin.username_charset"),
    ("用户名已存在", "admin.username_taken"),
    ("不能删除超级管理员", "admin.cannot_delete_super"),
    ("不能删除自己", "admin.cannot_delete_self"),
    ("不能修改超级管理员的状态", "admin.cannot_toggle_super"),
    ("不能修改自己的状态", "admin.cannot_toggle_self"),
    ("不能重置其他超级管理员的密码", "admin.cannot_reset_super_pwd"),
    // role
    ("角色名称不能为空", "role.name_required"),
    ("角色标识长度需 3-64", "role.code_length"),
    ("角色标识只能包含字母、数字、下划线、冒号", "role.code_charset"),
    ("角色标识已存在", "role.code_taken"),
    ("数据范围取值需在 1-5 之间", "role.bad_data_scope"),
    ("该角色已被管理员使用，不能删除", "role.in_use"),
    // menu
    ("菜单名称不能为空", "menu.name_required"),
    ("菜单类型只能是 M/C/F", "menu.bad_type"),
    ("不能把菜单挂到自己的子节点下", "menu.cycle"),
    ("上级菜单不存在", "menu.parent_missing"),
    ("请先删除子菜单", "menu.has_children"),
    ("该菜单已被角色引用", "menu.in_use"),
    // dept
    ("部门名称不能为空", "dept.name_required"),
    ("不能把部门挂到自己的子部门下", "dept.cycle"),
    ("上级部门不存在", "dept.parent_missing"),
    ("请先删除子部门", "dept.has_children"),
    ("该部门下还有管理员", "dept.has_admins"),
    // dict
    ("字典名称不能为空", "dict.type_name_required"),
    ("字典标签不能为空", "dict.label_required"),
    ("字典值不能为空", "dict.value_required"),
    ("字典编码已存在", "dict.code_taken"),
    ("该类型下字典值已存在", "dict.value_taken"),
    ("字典类型不存在", "dict.type_missing"),
    // job
    ("间隔必须是正整数秒", "job.bad_interval"),
    // notice
    ("公告标题不能为空", "notice.title_required"),
    ("公告内容不能为空", "notice.content_required"),
    // data scope
    ("超出你的数据权限范围", "scope.out_of_range"),
    ("不能授予数据范围更宽的角色", "scope.wider_role"),
    ("不能授予包含你没有的权限的角色", "scope.extra_perms"),
    ("角色不存在或已停用", "scope.role_unavailable"),
    // common（框架层兜底）
    ("数据已存在（唯一约束冲突）", "common.duplicate"),
    ("资源不存在", "common.not_found"),
    ("服务器内部错误", "common.internal"),
];

/// 按中文原文查稳定错误码（表里没有 → `None`，信封整个不带 `err`）。
fn code_of(msg: &str) -> Option<&'static str> {
    ERR_TABLE.iter().find(|(m, _)| *m == msg).map(|(_, e)| *e)
}

/// 字段键 → 中文标签：`args.field` 走字段键（前端按 `field.<键>` 翻译），
/// `msg` 仍拼中文，日志与 curl 读得懂。键查不到就原样显示键名。
fn field_label(key: &str) -> &str {
    match key {
        "nickname" => "昵称",
        "email" => "邮箱",
        "phone" => "手机号",
        "remark" => "备注",
        "name" => "名称",
        "code" => "标识",
        "leader" => "负责人",
        "contact" => "联系电话",
        "perm" => "权限标识",
        "path" => "路由路径",
        "component" => "组件路径",
        "icon" => "图标",
        "value" => "字典值",
        "label" => "字典标签",
        "title" => "标题",
        "content" => "内容",
        other => other,
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg, err, args) = match &self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone(), code_of(m), None),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "登录已失效，请重新登录".to_string(),
                Some("auth.unauthorized"),
                None,
            ),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone(), code_of(m), None),
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                "资源不存在".to_string(),
                Some("common.not_found"),
                None,
            ),
            ApiError::NotFoundMsg(m) => (StatusCode::NOT_FOUND, m.clone(), code_of(m), None),
            ApiError::Internal(m) => {
                // 细节只进日志，不外泄给客户端
                tracing::error!("内部错误: {m}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "服务器内部错误".to_string(),
                    Some("common.internal"),
                    None,
                )
            }
            ApiError::Coded { status, err, msg, args } => {
                (*status, msg.clone(), Some(*err), Some(args))
            }
        };
        envelope_opt(status, &msg, err, args)
    }
}

/// 统一错误信封 `{"code":…,"msg":…,"err":…,"args":…,"data":null}`。`ApiError` 之外的错误路径
/// （路由级 404/405 兜底，A1）也走它，保证契约「所有错误响应形状一致」没有例外。
/// `err` 为 `None` 时**整个不带这个键**（不是 `null`），前端回落中文 `msg`。
pub fn envelope(status: StatusCode, msg: &str, err: Option<&str>) -> Response {
    envelope_opt(status, msg, err, None)
}

fn envelope_opt(status: StatusCode, msg: &str, err: Option<&str>, args: Option<&Value>) -> Response {
    let mut body = json!({ "code": status.as_u16(), "msg": msg, "data": Value::Null });
    if let Some(err) = err {
        body["err"] = json!(err);
    }
    if let Some(args) = args {
        body["args"] = args.clone();
    }
    (status, Json(body)).into_response()
}

impl From<OrmError> for ApiError {
    fn from(e: OrmError) -> Self {
        match e {
            OrmError::DuplicateKey(_) => ApiError::BadRequest("数据已存在（唯一约束冲突）".into()),
            OrmError::NotFound => ApiError::NotFound,
            other => ApiError::Internal(format!("数据库错误: {other}")),
        }
    }
}

/// 成功信封：`{"code":0,"msg":"ok","data":…}`。
pub fn ok<T: serde::Serialize>(data: T) -> Json<Value> {
    Json(json!({ "code": 0, "msg": "ok", "data": data }))
}

/// 统一 JSON 提取器：把 axum 的 422 纯文本拒绝转成 `{code,msg,data}` 信封，
/// 让所有错误响应形状一致（前端拦截器按 msg 提示）。
pub struct AppJson<T>(pub T);

impl<S, T> axum::extract::FromRequest<S> for AppJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(v)) => Ok(AppJson(v)),
            Err(rej) => Err(ApiError::BadRequest(format!("请求体格式错误: {}", rej.body_text()))),
        }
    }
}

/// `Query` 版统一提取器：`?page=abc` 这类参数解析失败也回 JSON 信封，不是 400 纯文本。
pub struct AppQuery<T>(pub T);

impl<S, T> axum::extract::FromRequestParts<S> for AppQuery<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(v)) => Ok(AppQuery(v)),
            Err(rej) => Err(ApiError::BadRequest(format!("查询参数格式错误: {}", rej.body_text()))),
        }
    }
}

/// `Path` 版统一提取器：路径段类型不对（如 `/admins/abc`）也回 JSON 信封。
pub struct AppPath<T>(pub T);

impl<S, T> axum::extract::FromRequestParts<S> for AppPath<T>
where
    T: serde::de::DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(v)) => Ok(AppPath(v)),
            Err(rej) => Err(ApiError::BadRequest(format!("路径参数格式错误: {}", rej.body_text()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn err_table_shape_is_sane() {
        for (msg, err) in ERR_TABLE {
            assert!(!msg.is_empty(), "空 msg 行");
            let (module, reason) = err.split_once('.').unwrap_or_else(|| panic!("err 少了模块段: {err}"));
            for seg in [module, reason] {
                assert!(!seg.is_empty(), "err 段为空: {err}");
                assert!(
                    seg.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                    "err 只能是小写 + 下划线（契约：模块.原因）: {err}"
                );
            }
        }
        // 重复行会让「按 msg 查码」有歧义；err 重复则说明两条文案挤了同一个码（契约里一一对应）
        for (i, (msg, err)) in ERR_TABLE.iter().enumerate() {
            assert!(!ERR_TABLE[..i].iter().any(|(m, _)| m == msg), "重复 msg: {msg}");
            assert!(!ERR_TABLE[..i].iter().any(|(_, e)| e == err), "重复 err: {err}");
        }
    }

    #[test]
    fn missing_permission_carries_the_perm_code() {
        let ApiError::Coded { err, msg, args, .. } = ApiError::missing_permission("system:dict:add")
        else {
            panic!("应是 Coded")
        };
        assert_eq!(err, "auth.forbidden", "契约里 403 默认就这一条，用户最常撞见");
        assert_eq!(msg, "缺少权限：system:dict:add", "msg 仍是中文原文");
        assert_eq!(args["code"], "system:dict:add", "args.code 是缺的权限码原始值");
    }

    #[test]
    fn code_of_hits_known_and_misses_unknown() {
        assert_eq!(code_of("用户名或密码错误"), Some("auth.bad_credentials"));
        assert_eq!(code_of("字典类型不存在"), Some("dict.type_missing"));
        assert_eq!(code_of("资源不存在"), Some("common.not_found"));
        assert_eq!(code_of("缺少权限：system:dict:add"), None, "动态文案没有稳定码");
    }

    #[test]
    fn too_long_carries_field_key_and_label() {
        let e = ApiError::too_long("remark", 255);
        let ApiError::Coded { err, msg, args, .. } = &e else { panic!("应是 Coded: {e:?}") };
        assert_eq!(*err, "common.too_long");
        assert_eq!(msg, "备注 长度不能超过 255 个字符", "msg 仍是中文原文（日志/curl 用）");
        assert_eq!(args["field"], "remark", "args.field 传字段键，不是中文标签");
        assert_eq!(args["max"], 255);
        // 表里没有的键：原样显示键名，宁可露出 remark 也别空着
        let ApiError::Coded { msg, .. } = ApiError::too_long("whatever", 1) else { unreachable!() };
        assert_eq!(msg, "whatever 长度不能超过 1 个字符");
    }

    /// 信封：`err` 在就有、不在就整个缺键；`args` 只在带参码出现。
    #[tokio::test]
    async fn envelope_omits_err_key_when_absent() {
        let body = |r: Response| async move {
            let bytes = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        };
        let with = body(ApiError::BadRequest("用户名或密码错误".into()).into_response()).await;
        assert_eq!(with["err"], "auth.bad_credentials");
        assert!(with.get("args").is_none(), "静态码不带 args: {with}");

        let without = body(ApiError::BadRequest("请求体格式错误: xxx".into()).into_response()).await;
        assert!(without.get("err").is_none(), "没有稳定码就整个不带 err 键: {without}");
        assert_eq!(without["code"], 400);
    }
}
