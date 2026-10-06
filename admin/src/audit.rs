// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 操作日志中间件（B5）：写操作自动落一条审计记录。
//! 审计失败绝不影响业务请求（只打错误日志）。
use crate::api::auth::client_ip;
use crate::auth::verify_token;
use crate::models::{Admin, AuditLog};
use crate::state::AppState;
use crate::util::now;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use serde_json::Value;
use std::time::Instant;

/// 失败响应体上限：错误信封都是几百字节，超出说明不是我们的信封，不读。
const ERR_BODY_LIMIT: usize = 64 * 1024;

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
    let duration_ms = started.elapsed().as_millis() as u64;
    let status = resp.status().is_success();

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
    let mut row = AuditLog {
        id: 0,
        admin_id,
        username,
        module,
        action,
        method: method.to_string(),
        path: path.chars().take(255).collect(),
        status: if status { 1 } else { 0 },
        msg: msg.chars().take(255).collect(),
        duration_ms,
        ip,
        created_at: now(),
    };
    if let Err(e) = state.db.insert(&mut row).await {
        tracing::error!("写操作日志失败: {e}");
    }
    resp
}

/// 从 Authorization 头解出操作者：只解 token + 一次按 id 查用户名（只发生在写操作上）。
/// 凭据无效不拦请求（各接口自己的 Auth 提取器会给 401），这里退化成匿名记录。
async fn actor(state: &AppState, headers: &axum::http::HeaderMap) -> (u64, String) {
    let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return (0, String::new());
    };
    let Ok(claims) = verify_token(token, &state.cfg) else {
        return (0, String::new());
    };
    let name = match Admin::query().filter_eq("id", claims.sub) {
        Ok(qs) => qs
            .fetch_one(&state.db)
            .await
            .ok()
            .flatten()
            .map(|a| a.username)
            .unwrap_or_default(),
        Err(_) => String::new(),
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

        ("DELETE", ["login-logs"]) => ("loginlog", "清空登录记录"),
        ("DELETE", ["audit-logs"]) => ("auditlog", "清空操作日志"),

        ("POST", ["auth", "logout"]) => ("auth", "退出登录"),
        ("PUT", ["auth", "password"]) => ("auth", "修改密码"),
        ("PUT", ["auth", "profile"]) => ("auth", "修改个人资料"),
        ("POST", ["auth", "avatar"]) => ("auth", "上传头像"),

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
            module_action(&Method::POST, "/api/v1/auth/avatar"),
            ("auth".into(), "上传头像".into())
        );
    }

    #[test]
    fn unknown_path_falls_back_to_other() {
        let (m, a) = module_action(&Method::POST, "/api/v1/nope/1");
        assert_eq!(m, "other");
        assert_eq!(a, "POST /api/v1/nope/1");
    }
}
