// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::error::ApiError;
use crate::models::{Admin, Menu, Role};
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use jwt_rust::config::JwtConfig;
use jwt_rust::storage::TokenStorage;
use jwt_rust::{Jwt, JwtError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use crate::relations::RelationsExt;
use crate::api::ids_to_values;

/// payload 里本项目自己带的东西：iss/aud/iat/nbf/exp/jti 由 jwt-rust 插入并校验。
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// admin.id
    pub sub: i64,
    /// token_version：管理员改密/被禁用后自增，旧 token 立即失效
    pub ver: i32,
}

/// 签发/校验两端共用的标记（issuer 与 audience 同值即可）
const JWT_ISSUER: &str = "bee-rust-admin";

/// 空黑名单。本项目的失效靠 claims 里的 `ver`（= admin.token_version）：改密/禁用/登出
/// 都是对库里 token_version 的一次 UPDATE，全量失效比逐个 token 拉黑彻底，所以不记黑名单。
/// 而 `Jwt::new` 强制要一个 `TokenStorage`，给个永远「不在黑名单」的空实现。
struct NoBlacklist;

impl TokenStorage for NoBlacklist {
    fn blacklist(&self, _jti: &str, _expire_time: i64) -> Result<bool, JwtError> {
        Ok(true)
    }
    fn is_blacklisted(&self, _jti: &str) -> Result<bool, JwtError> {
        Ok(false)
    }
    fn cleanup(&self) -> Result<bool, JwtError> {
        Ok(true)
    }
}

/// 按 `[jwt]` 配置组装 Jwt（含密钥与配置，进程内建一次放进 AppState）。
pub fn build_jwt(cfg: &AppConfig) -> Result<Jwt, JwtError> {
    let config = JwtConfig {
        secret_key: cfg.jwt_secret.clone(),
        algorithm: "HS256".into(),
        issuer: JWT_ISSUER.into(),
        audience: JWT_ISSUER.into(),
        default_expire: expire_secs(cfg).max(0) as u64,
        ..JwtConfig::default()
    };
    Jwt::new(config, Arc::new(NoBlacklist))
}

/// `[jwt] expire_hours` → 秒（签发有效期，也是回给前端的 `expires_in`）
pub fn expire_secs(cfg: &AppConfig) -> i64 {
    cfg.jwt_expire_hours * 3600
}

/// 签发 token，返回 (token, 有效期秒数)。
pub fn sign_token(
    jwt: &Jwt,
    admin_id: i64,
    ver: i32,
    expire_secs: i64,
) -> Result<(String, i64), ApiError> {
    let claims = Claims { sub: admin_id, ver };
    // 负数直接用 `as u64` 会绕成天文数字（token 永不过期），先夹到 0 = 立即过期
    let token = jwt
        .encode(&claims, Some(expire_secs.max(0) as u64))
        .map_err(|e| ApiError::internal(format!("签发 token 失败: {e}")))?;
    Ok((token, expire_secs))
}

pub fn verify_token(jwt: &Jwt, token: &str) -> Result<Claims, ApiError> {
    jwt.decode_as::<Claims>(token).map_err(|_| ApiError::Unauthorized)
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
        Err(ApiError::missing_permission(code))
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

        let claims = verify_token(&state.jwt, token)?;

        let admin = Admin::query()
            .filter_eq("id", claims.sub)
            .map_err(ApiError::from)?
            .one(&state.db)
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
                .filter_in("id", &ids_to_values(&role_ids))
                .map_err(ApiError::from)?
                .all(&state.db)
                .await
                .map_err(ApiError::from)?
        };

        let perms = if is_super {
            HashSet::from(["*:*:*".to_string()])
        } else {
            let mut menu_ids: Vec<i64> = Vec::new();
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
                    .filter_in("id", &ids_to_values(&menu_ids))
                    .map_err(ApiError::from)?
                    // 禁用的菜单/按钮即时收权：状态改了不用重启，下一请求就少这个码
                    .filter_eq("status", 1)
                    .map_err(ApiError::from)?
                    .all(&state.db)
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
            captcha: true,
            retain_days: 90,
            job_enabled: true,
            hashids_salt: "test-salt".into(),
            hashids_min_len: 8,
            snowflake_worker: 0,
            snowflake_dc: 0,
            encrypt_key: "base64:YmVlLWFkbWluLXRlc3Qta2V5LTMyLWJ5dGVzLW9rISE=".into(),
        }
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let jwt = build_jwt(&cfg()).unwrap();
        let (token, ttl) = sign_token(&jwt, 7, 3, expire_secs(&cfg())).unwrap();
        assert_eq!(ttl, 86400);
        let claims = verify_token(&jwt, &token).unwrap();
        assert_eq!(claims.sub, 7);
        assert_eq!(claims.ver, 3);
    }

    #[test]
    fn tampered_token_rejected() {
        let jwt = build_jwt(&cfg()).unwrap();
        let (token, _) = sign_token(&jwt, 7, 0, expire_secs(&cfg())).unwrap();
        let mut other = cfg();
        other.jwt_secret = "9999999999999999999999999999999999".into();
        assert!(verify_token(&build_jwt(&other).unwrap(), &token).is_err());
        assert!(verify_token(&jwt, "garbage").is_err());
    }
}
