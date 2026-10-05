// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::auth::{Auth, sign_token};
use crate::error::{ApiError, AppJson, ok};
use crate::models::{Admin, LoginLog, Menu};
use crate::state::AppState;
use crate::util::{hash_password, now, verify_password};
use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct ChangePasswordBody {
    pub old_password: String,
    pub new_password: String,
}

/// 客户端 IP：nginx 透传的 X-Real-IP → X-Forwarded-For 首个 → unknown。
/// 头部由客户端可控，截到 IPv6 文本最长 45 字符，避免写库超长报错。
pub fn client_ip(headers: &HeaderMap) -> String {
    let raw = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| {
            headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    raw.chars().take(45).collect()
}

fn user_agent(headers: &HeaderMap) -> String {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(255)
        .collect()
}

pub async fn write_login_log(
    state: &AppState,
    admin_id: u64,
    username: &str,
    headers: &HeaderMap,
    status: i8,
    msg: &str,
) {
    let mut log = LoginLog {
        id: 0,
        admin_id,
        username: username.chars().take(64).collect(),
        ip: client_ip(headers),
        user_agent: user_agent(headers),
        status,
        msg: msg.chars().take(255).collect(),
        created_at: now(),
    };
    if let Err(e) = state.db.insert(&mut log).await {
        // 登录记录写失败不能影响登录本身
        tracing::error!("写登录记录失败: {e}");
    }
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    AppJson(body): AppJson<LoginBody>,
) -> Result<Json<Value>, ApiError> {
    if body.username.trim().is_empty() || body.password.is_empty() {
        return Err(ApiError::BadRequest("用户名和密码不能为空".into()));
    }

    let admin = Admin::query()
        .filter_eq("username", body.username.trim())
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?;

    let Some(admin) = admin else {
        write_login_log(&state, 0, &body.username, &headers, 0, "用户不存在").await;
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    };
    if !verify_password(&body.password, &admin.password) {
        write_login_log(&state, admin.id, &admin.username, &headers, 0, "密码错误").await;
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    }
    if admin.status != 1 {
        write_login_log(&state, admin.id, &admin.username, &headers, 0, "账号已禁用").await;
        return Err(ApiError::BadRequest("账号已被禁用".into()));
    }

    let (token, expires_in) = sign_token(admin.id, admin.token_version, &state.cfg)?;

    let mut updated = admin.clone();
    updated.last_login_at = Some(now());
    updated.last_login_ip = client_ip(&headers);
    updated.updated_at = now();
    state.db.update(&updated).await.map_err(ApiError::from)?;

    write_login_log(&state, admin.id, &admin.username, &headers, 1, "登录成功").await;

    Ok(ok(json!({
        "token": token,
        "expires_in": expires_in,
        "user": {
            "id": admin.id,
            "username": admin.username,
            "nickname": admin.nickname,
            "avatar": admin.avatar,
            "is_super": admin.is_super == 1,
            "dept_id": admin.dept_id,
        }
    })))
}

pub async fn logout(
    State(state): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let mut admin = auth.admin.clone();
    admin.token_version += 1; // 当前 token 立即失效
    admin.updated_at = now();
    state.db.update(&admin).await.map_err(ApiError::from)?;
    write_login_log(&state, admin.id, &admin.username, &headers, 1, "退出登录").await;
    Ok(ok(Value::Null))
}

pub async fn profile(State(_state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let mut perms: Vec<&String> = auth.perms.iter().collect();
    perms.sort();
    let roles: Vec<&str> = auth.roles.iter().map(|r| r.code.as_str()).collect();
    Ok(ok(json!({
        "user": {
            "id": auth.admin.id,
            "username": auth.admin.username,
            "nickname": auth.admin.nickname,
            "avatar": auth.admin.avatar,
            "is_super": auth.is_super,
            "dept_id": auth.admin.dept_id,
        },
        "roles": roles,
        "perms": perms,
    })))
}

/// 当前用户的菜单树（只含目录/菜单；超管全量，否则按角色勾选）。
pub async fn menus(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let all = Menu::query()
        .filter_eq("status", 1)
        .map_err(ApiError::from)?
        .filter_eq("visible", 1)
        .map_err(ApiError::from)?
        .order_by("sort ASC, id ASC")
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;

    let allowed: Option<Vec<u64>> = if auth.is_super {
        None
    } else {
        let mut ids: Vec<u64> = Vec::new();
        for rid in auth.roles.iter().filter(|r| r.status == 1).map(|r| r.id) {
            ids.extend(
                state
                    .db
                    .get_relations("role_menu", ("role_id", rid), "menu_id")
                    .await
                    .map_err(ApiError::from)?,
            );
        }
        ids.sort_unstable();
        ids.dedup();
        // 客户端只提交子节点时（antd 半选不落库），补全祖先，否则拼不出树、侧边栏为空
        let parent_of: HashMap<u64, u64> = all.iter().map(|m| (m.id, m.parent_id)).collect();
        Some(crate::api::with_ancestors(&parent_of, &ids))
    };

    let visible: Vec<&Menu> = all
        .iter()
        .filter(|m| m.menu_type == "M" || m.menu_type == "C")
        .filter(|m| match &allowed {
            None => true,
            Some(ids) => ids.contains(&m.id),
        })
        .collect();

    fn build(parent: u64, nodes: &[&Menu]) -> Vec<Value> {
        nodes
            .iter()
            .filter(|m| m.parent_id == parent)
            .map(|m| {
                json!({
                    "id": m.id,
                    "parent_id": m.parent_id,
                    "name": m.name,
                    "path": m.path,
                    "icon": m.icon,
                    "children": build(m.id, nodes),
                })
            })
            .collect()
    }

    Ok(ok(build(0, &visible)))
}

pub async fn change_password(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<ChangePasswordBody>,
) -> Result<Json<Value>, ApiError> {
    if body.new_password.len() < 6 {
        return Err(ApiError::BadRequest("新密码至少 6 位".into()));
    }
    if !verify_password(&body.old_password, &auth.admin.password) {
        return Err(ApiError::BadRequest("原密码错误".into()));
    }
    let mut admin = auth.admin.clone();
    admin.password = hash_password(&body.new_password);
    admin.token_version += 1; // 全端下线，需重新登录
    admin.updated_at = now();
    state.db.update(&admin).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ip_priority_and_truncation() {
        let mut h = HeaderMap::new();
        // 无任何头 → unknown
        assert_eq!(client_ip(&h), "unknown");
        // X-Forwarded-For 取第一个
        let _ = h.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());
        assert_eq!(client_ip(&h), "1.2.3.4");
        // X-Real-IP 优先
        let _ = h.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        assert_eq!(client_ip(&h), "9.9.9.9");
        // 客户端可控的超长头截到 45 字符（IPv6 文本上限），不会写库超长
        let _ = h.insert("x-real-ip", "x".repeat(300).parse().unwrap());
        assert_eq!(client_ip(&h).len(), 45);
    }
}
