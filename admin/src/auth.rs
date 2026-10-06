// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::error::ApiError;
use crate::models::{Admin, Menu, Role};
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// admin.id
    pub sub: u64,
    /// token_version：管理员改密/被禁用后自增，旧 token 立即失效
    pub ver: i32,
    pub iat: i64,
    pub exp: i64,
}

/// 签发 token，返回 (token, 有效期秒数)。
pub fn sign_token(admin_id: u64, ver: i32, cfg: &AppConfig) -> Result<(String, i64), ApiError> {
    let now = chrono::Utc::now().timestamp();
    let exp = now + cfg.jwt_expire_hours * 3600;
    let claims = Claims { sub: admin_id, ver, iat: now, exp };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(cfg.jwt_secret.as_bytes()),
    )
    .map_err(|e| ApiError::internal(format!("签发 token 失败: {e}")))?;
    Ok((token, cfg.jwt_expire_hours * 3600))
}

pub fn verify_token(token: &str, cfg: &AppConfig) -> Result<Claims, ApiError> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(cfg.jwt_secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )
    .map(|d| d.claims)
    .map_err(|_| ApiError::Unauthorized)
}

/// 已认证用户：管理员本体 + 角色 + 权限码。
#[derive(Debug)]
pub struct Auth {
    pub admin: Admin,
    pub roles: Vec<Role>,
    pub perms: HashSet<String>,
    pub is_super: bool,
}

impl Auth {
    /// 逐接口权限校验；超管直接放行。
    pub fn require(&self, code: &str) -> Result<(), ApiError> {
        if self.is_super || self.perms.contains(code) {
            return Ok(());
        }
        Err(ApiError::Forbidden(format!("缺少权限：{code}")))
    }
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;

        let claims = verify_token(token, &state.cfg)?;

        let admin = Admin::query()
            .filter_eq("id", claims.sub)
            .map_err(ApiError::from)?
            .fetch_one(&state.db)
            .await
            .map_err(ApiError::from)?
            .ok_or(ApiError::Unauthorized)?;

        if admin.status != 1 || admin.token_version != claims.ver {
            return Err(ApiError::Unauthorized);
        }

        let is_super = admin.is_super == 1;
        let role_ids = state
            .db
            .get_relations("admin_role", ("admin_id", admin.id), "role_id")
            .await
            .map_err(ApiError::from)?;

        let roles = if role_ids.is_empty() {
            Vec::new()
        } else {
            Role::query()
                .filter_in("id", role_ids)
                .map_err(ApiError::from)?
                .fetch_all(&state.db)
                .await
                .map_err(ApiError::from)?
        };

        let perms = if is_super {
            HashSet::from(["*:*:*".to_string()])
        } else {
            let mut menu_ids: Vec<u64> = Vec::new();
            for rid in roles.iter().filter(|r| r.status == 1).map(|r| r.id) {
                let ids = state
                    .db
                    .get_relations("role_menu", ("role_id", rid), "menu_id")
                    .await
                    .map_err(ApiError::from)?;
                menu_ids.extend(ids);
            }
            menu_ids.sort_unstable();
            menu_ids.dedup();
            if menu_ids.is_empty() {
                HashSet::new()
            } else {
                Menu::query()
                    .filter_in("id", menu_ids)
                    .map_err(ApiError::from)?
                    // 禁用的菜单/按钮即时收权：状态改了不用重启，下一请求就少这个码
                    .filter_eq("status", 1)
                    .map_err(ApiError::from)?
                    .fetch_all(&state.db)
                    .await
                    .map_err(ApiError::from)?
                    .into_iter()
                    .filter(|m| !m.perm.is_empty())
                    .map(|m| m.perm)
                    .collect()
            }
        };

        Ok(Auth { admin, roles, perms, is_super })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AppConfig {
        AppConfig {
            app_name: "t".into(),
            http_addr: "127.0.0.1:0".into(),
            log_level: "warn".into(),
            db_dsn: "mysql://x".into(),
            jwt_secret: "0123456789012345678901234567890123".into(),
            jwt_expire_hours: 24,
            initial_admin_password: "admin123".into(),
            upload_dir: "uploads".into(),
            max_fail: 5,
            lock_minutes: 10,
            retain_days: 90,
        }
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let (token, ttl) = sign_token(7, 3, &cfg()).unwrap();
        assert_eq!(ttl, 86400);
        let claims = verify_token(&token, &cfg()).unwrap();
        assert_eq!(claims.sub, 7);
        assert_eq!(claims.ver, 3);
    }

    #[test]
    fn tampered_token_rejected() {
        let (token, _) = sign_token(7, 0, &cfg()).unwrap();
        let mut other = cfg();
        other.jwt_secret = "9999999999999999999999999999999999".into();
        assert!(verify_token(&token, &other).is_err());
        assert!(verify_token("garbage", &cfg()).is_err());
    }
}
