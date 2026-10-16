// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 操作日志中间件（B5）：写操作自动落一条审计记录。
//! 审计失败绝不影响业务请求（只打错误日志）。
use crate::api::auth::client_ip;
use crate::auth::verify_token;
use crate::models::{Admin, AuditLog};
use crate::state::AppState;
use bee_orm::Model;
use crate::util::now;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use axum::response::{IntoResponseParts, ResponseParts};
use serde_json::{Value, json};
use std::convert::Infallible;
use std::time::Instant;

/// 失败响应体上限：错误信封都是几百字节，超出说明不是我们的信封，不读。
const ERR_BODY_LIMIT: usize = 64 * 1024;

/// 变更摘要（项 8）：handler 把它塞进响应扩展（`(diff.done(), ok(…))`），
/// 中间件在 `next.run()` 之后取出来落进 `audit_log.changes`。
/// 走扩展而不是让 handler 自己写审计行：那种做法同一请求会落两行。
#[derive(Clone, Debug)]
pub struct AuditDetail(pub Value);

impl IntoResponseParts for AuditDetail {
    type Error = Infallible;
    fn into_response_parts(self, mut res: ResponseParts) -> Result<ResponseParts, Self::Error> {
        res.extensions_mut().insert(self);
        Ok(res)
    }
}

/// 变更摘要收集器：`field()` 只收真的变了的字段，`masked()` 给密文列用。
/// 用法：`Diff::default().field("nickname", old.nickname, a.nickname.clone()).done()`。
#[derive(Default)]
pub struct Diff(serde_json::Map<String, Value>);

impl Diff {
    pub fn field(mut self, name: &'static str, old: impl Into<Value>, new: impl Into<Value>) -> Self {
        let (old, new): (Value, Value) = (old.into(), new.into());
        if old != new {
            self.0.insert(name.into(), json!({ "old": old, "new": new }));
        }
        self
    }

    /// 密文列（email / phone）：只记「改过」，值（明文与密文）都不进摘要 ——
    /// 记明文等于抵消落库加密，记密文每次重加密都不同、没有可比性。
    pub fn masked(mut self, name: &'static str, changed: bool) -> Self {
        if changed {
            self.0.insert(name.into(), json!({ "changed": true }));
        }
        self
    }

    pub fn done(self) -> AuditDetail {
        AuditDetail(Value::Object(self.0))
    }
}

/// 密文列改动判定：比对**明文**（新值来自请求体，旧值在库里是密文，先解出来）。
/// 解不开（存量脏数据）当改过 —— 宁可多记一次「已变更」。
pub fn masked_changed(guard: &encryptable::guard::Guard, old_cipher: &str, new_plain: &str) -> bool {
    crate::crypto::plain(guard, old_cipher).map(|p| p != new_plain).unwrap_or(true)
}

pub async fn audit_mw(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    // 只记写操作；健康检查与登录（本身有 login_log）不记
    if !matches!(method, Method::POST | Method::PUT | Method::DELETE)
        || path == "/api/v1/health"
        || path == "/api/v1/auth/login"
    {
        return next.run(req).await;
    }

    let ip = client_ip(req.headers());
    let (admin_id, username) = actor(&state, req.headers()).await;

    let started = Instant::now();
    let mut resp = next.run(req).await;
    let duration_ms = started.elapsed().as_millis() as i64;
    let status = resp.status().is_success();
    // 变更摘要：handler 挂了才有（失败路径本来也不会挂）。趁 body 还没被读走先取出来。
    let changes = resp
        .extensions()
        .get::<AuditDetail>()
        .map(|d| d.0.to_string())
        .unwrap_or_default();

    // 失败时取出信封里的 msg 留档。成功响应（含 CSV 大附件）不碰 body。
    let mut msg = String::new();
    if !status {
        let (parts, body) = resp.into_parts();
        match axum::body::to_bytes(body, ERR_BODY_LIMIT).await {
            Ok(bytes) => {
                msg = serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .and_then(|v| v["msg"].as_str().map(str::to_string))
                    .unwrap_or_default();
                resp = Response::from_parts(parts, Body::from(bytes));
            }
            // 读失败只能放弃留档（body 已消费，无法还原），业务响应照常返回
            Err(e) => {
                tracing::error!("审计读取失败响应体失败: {e}");
                resp = Response::from_parts(parts, Body::empty());
            }
        }
    }

    let (module, action) = module_action(&method, &path);
    // 发号失败只影响留档（业务响应照常返回）——与写日志失败同一处理口径
    match state.next_id() {
        Ok(id) => {
            let row = AuditLog {
                id,
                admin_id,
                username,
                module,
                action,
                method: method.to_string(),
                path: path.chars().take(255).collect(),
                status: if status { 1 } else { 0 },
                msg: msg.chars().take(255).collect(),
                changes,
                duration_ms,
                ip,
                created_at: now(),
            };
            if let Err(e) = row.insert(&state.db).await {
                tracing::error!("写操作日志失败: {e}");
            }
        }
        Err(e) => tracing::error!("生成操作日志 id 失败: {e:?}"),
    }
    resp
}

/// 从 Authorization 头解出操作者：只解 token + 一次按 id 查用户名（只发生在写操作上）。
/// 凭据无效不拦请求（各接口自己的 Auth 提取器会给 401），这里退化成匿名记录。
pub(crate) async fn actor(state: &AppState, headers: &axum::http::HeaderMap) -> (i64, String) {
    let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return (0, String::new());
    };
    let Ok(claims) = verify_token(&state.jwt, token) else {
        return (0, String::new());
    };
    let qs = match Admin::query().filter_eq("id", claims.sub) {
        Ok(qs) => qs,
        Err(e) => {
            tracing::error!("审计构造操作者查询失败 (id={}): {e}", claims.sub);
            return (claims.sub, String::new());
        }
    };
    let name = match qs.one(&state.db).await {
        Ok(Some(a)) => a.username,
        // 管理员被删但 token 还没过期：留匿名记录即可，不是错误
        Ok(None) => String::new(),
        Err(e) => {
            tracing::error!("审计查询操作者失败 (id={}): {e}", claims.sub);
            String::new()
        }
    };
    (claims.sub, name)
}

/// path + method →（模块, 动作）。只列会改数据的接口（中间件也只记 POST/PUT/DELETE）；
/// 未列出的路径归 `other`，动作用 `METHOD /path`，方便事后发现可疑探测。
fn module_action(method: &Method, path: &str) -> (String, String) {
    let rest = path.strip_prefix("/api/v1/").unwrap_or(path);
    let seg: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    let (module, action) = match (method.as_str(), seg.as_slice()) {
        ("POST", ["admins"]) => ("admin", "新增管理员"),
        ("PUT", ["admins", _, "status"]) => ("admin", "修改管理员状态"),
        ("PUT", ["admins", _, "password"]) => ("admin", "重置管理员密码"),
        ("PUT", ["admins", _, "roles"]) => ("admin", "分配管理员角色"),
        ("PUT", ["admins", _]) => ("admin", "编辑管理员"),
        ("DELETE", ["admins", _]) => ("admin", "删除管理员"),

        ("POST", ["roles"]) => ("role", "新增角色"),
        ("PUT", ["roles", _, "menus"]) => ("role", "分配角色菜单"),
        ("PUT", ["roles", _, "depts"]) => ("role", "分配角色部门"),
        ("PUT", ["roles", _]) => ("role", "编辑角色"),
        ("DELETE", ["roles", _]) => ("role", "删除角色"),

        ("POST", ["menus"]) => ("menu", "新增菜单"),
        ("PUT", ["menus", _]) => ("menu", "编辑菜单"),
        ("DELETE", ["menus", _]) => ("menu", "删除菜单"),

        ("POST", ["depts"]) => ("dept", "新增部门"),
        ("PUT", ["depts", _]) => ("dept", "编辑部门"),
        ("DELETE", ["depts", _]) => ("dept", "删除部门"),

        ("POST", ["dicts"]) => ("dict", "新增字典类型"),
        ("PUT", ["dicts", _]) => ("dict", "编辑字典类型"),
        ("DELETE", ["dicts", _]) => ("dict", "删除字典类型"),
        ("POST", ["dict-items"]) => ("dict", "新增字典项"),
        ("PUT", ["dict-items", _]) => ("dict", "编辑字典项"),
        ("DELETE", ["dict-items", _]) => ("dict", "删除字典项"),

        ("POST", ["configs"]) => ("config", "新增系统配置"),
        ("PUT", ["configs", _]) => ("config", "编辑系统配置"),
        ("DELETE", ["configs", _]) => ("config", "删除系统配置"),

        // CSV 导入（项 3）：kind 在 query 上（这里只看路径），三种导入同一条动作名 ——
        // 具体种别看审计行的 path 或报告，别为此拆三条（拆了也没法从路径分辨）
        ("POST", ["imports"]) => ("import", "导入数据"),

        ("POST", ["exports"]) => ("export", "发起导出"),
        ("DELETE", ["exports", _]) => ("export", "删除导出任务"),

        ("PUT", ["jobs", _]) => ("job", "编辑定时任务"),
        ("POST", ["jobs", _, "run"]) => ("job", "手动执行定时任务"),

        ("POST", ["notices"]) => ("notice", "新增公告"),
        ("PUT", ["notices", _]) => ("notice", "编辑公告"),
        ("POST", ["notices", _, "read"]) => ("notice", "标记公告已读"),
        ("DELETE", ["notices", _]) => ("notice", "删除公告"),

        // 通知渠道（项 6）：存配置、发测试邮件都会改库/外发，要留痕。
        // 摘要里只有非口令字段 —— 口令走 `Diff::masked`，只记「已变更」（见 api::notify）。
        ("PUT", ["notify", "config"]) => ("notify", "保存通知渠道配置"),
        ("POST", ["notify", "test-email"]) => ("notify", "发送测试邮件"),

        ("DELETE", ["login-logs"]) => ("loginlog", "清空登录记录"),
        ("DELETE", ["audit-logs"]) => ("auditlog", "清空操作日志"),

        ("POST", ["auth", "logout"]) => ("auth", "退出登录"),
        ("POST", ["auth", "logout-others"]) => ("auth", "退出其他设备"),
        ("PUT", ["auth", "password"]) => ("auth", "修改密码"),
        ("PUT", ["auth", "profile"]) => ("auth", "修改个人资料"),
        // 项 7：这几条都会改库（密钥 / 开关 / 设备行），要留痕。
        // 摘要一律为空 —— 密钥与验证码都不该进审计（密钥还是密文落库的）。
        ("POST", ["auth", "totp", "setup"]) => ("auth", "生成动态验证码密钥"),
        ("POST", ["auth", "totp", "enable"]) => ("auth", "绑定动态验证码"),
        ("POST", ["auth", "totp", "disable"]) => ("auth", "解绑动态验证码"),
        ("DELETE", ["auth", "devices", _]) => ("auth", "踢出登录设备"),
        // 头像分片上传（4b）：preprocess 与每一片 chunk 各留一条 —— 传超时/中断时
        // 「传到第几片」在日志里看得见（base64 那条老路由是一次性的，已删）
        ("POST", ["avatar", "upload", "preprocess"]) => ("auth", "上传头像"),
        ("POST", ["avatar", "upload", "chunk"]) => ("auth", "上传头像"),

        // 附件中心（项 5）：同头像，preprocess 与每一片 chunk 各留一条；
        // 改关联（含解绑）与删除各一条 —— 「谁把配图挂到公告上/删了」必须查得到
        ("POST", ["attachments", "upload", "preprocess"]) => ("attach", "上传附件"),
        ("POST", ["attachments", "upload", "chunk"]) => ("attach", "上传附件"),
        ("PUT", ["attachments", _]) => ("attach", "关联附件"),
        ("DELETE", ["attachments", _]) => ("attach", "删除附件"),

        _ => return ("other".into(), format!("{} {}", method, path)),
    };
    (module.into(), action.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_paths_to_module_and_action() {
        assert_eq!(module_action(&Method::POST, "/api/v1/admins"), ("admin".into(), "新增管理员".into()));
        assert_eq!(
            module_action(&Method::PUT, "/api/v1/admins/7/status"),
            ("admin".into(), "修改管理员状态".into())
        );
        assert_eq!(
            module_action(&Method::DELETE, "/api/v1/login-logs"),
            ("loginlog".into(), "清空登录记录".into())
        );
        assert_eq!(
            module_action(&Method::POST, "/api/v1/avatar/upload/preprocess"),
            ("auth".into(), "上传头像".into())
        );
        assert_eq!(
            module_action(&Method::POST, "/api/v1/avatar/upload/chunk"),
            ("auth".into(), "上传头像".into())
        );
        assert_eq!(
            module_action(&Method::DELETE, "/api/v1/dicts/3"),
            ("dict".into(), "删除字典类型".into())
        );
        assert_eq!(
            module_action(&Method::POST, "/api/v1/dict-items"),
            ("dict".into(), "新增字典项".into())
        );
        assert_eq!(
            module_action(&Method::PUT, "/api/v1/configs/5"),
            ("config".into(), "编辑系统配置".into())
        );
        assert_eq!(
            module_action(&Method::DELETE, "/api/v1/configs/5"),
            ("config".into(), "删除系统配置".into())
        );
        assert_eq!(
            module_action(&Method::PUT, "/api/v1/jobs/2"),
            ("job".into(), "编辑定时任务".into())
        );
        assert_eq!(
            module_action(&Method::POST, "/api/v1/jobs/2/run"),
            ("job".into(), "手动执行定时任务".into())
        );
        assert_eq!(
            module_action(&Method::POST, "/api/v1/exports"),
            ("export".into(), "发起导出".into())
        );
        assert_eq!(
            module_action(&Method::DELETE, "/api/v1/exports/7"),
            ("export".into(), "删除导出任务".into())
        );
        assert_eq!(
            module_action(&Method::POST, "/api/v1/notices/9/read"),
            ("notice".into(), "标记公告已读".into())
        );
        assert_eq!(
            module_action(&Method::DELETE, "/api/v1/notices/9"),
            ("notice".into(), "删除公告".into())
        );
    }

    /// 摘要只收真的变了的字段；密文列只记 `{"changed":true}`。
    #[test]
    fn diff_keeps_only_changed_fields() {
        let d = Diff::default()
            .field("same", "a", "a")
            .field("nickname", "旧", "新")
            .field("sort", 1, 2)
            .masked("email", false)
            .masked("phone", true)
            .done();
        assert_eq!(
            d.0,
            json!({
                "nickname": {"old": "旧", "new": "新"},
                "sort": {"old": 1, "new": 2},
                "phone": {"changed": true},
            })
        );
    }

    /// 密文列判定比的是**明文**：同一明文重新加密（密文不同）不算改过。
    #[test]
    fn masked_changed_compares_plaintext_not_ciphertext() {
        // 与 conf/app.conf.test 同源的固定密钥（32 字节）
        let g = crate::crypto::build_guard("base64:YmVlLWFkbWluLXRlc3Qta2V5LTMyLWJ5dGVzLW9rISE=").unwrap();
        let cipher = crate::crypto::write(&g, "a@b.c").unwrap();
        // 随机 nonce：同一明文两次加密密文不同 —— 所以判定只能比明文
        assert_ne!(cipher, crate::crypto::write(&g, "a@b.c").unwrap(), "密文每次都不一样");
        assert!(!masked_changed(&g, &cipher, "a@b.c"), "明文没变就不算改过");
        assert!(masked_changed(&g, &cipher, "c@d.e"), "明文变了要记「已变更」");
        assert!(masked_changed(&g, "不是密文", "x"), "解不开当改过（宁可多记）");
    }

    #[test]
    fn unknown_path_falls_back_to_other() {
        let (m, a) = module_action(&Method::POST, "/api/v1/nope/1");
        assert_eq!(m, "other");
        assert_eq!(a, "POST /api/v1/nope/1");
    }
}
