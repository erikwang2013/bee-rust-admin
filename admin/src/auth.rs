// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::error::ApiError;
use crate::models::{Admin, AdminLoginDevice, Menu, Role};
use crate::state::AppState;
use axum::extract::FromRequestParts;
use bee_orm::Model;
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
    /// 登录设备行的 id（`admin_login_device`，项 7）；0 = 没有设备概念。
    /// `default` 是**升级兼容**：老 token 里没有这个字段，解出来是 0，
    /// 于是不按设备校验 —— 不然一升级重启就把所有在线的人踢下线。
    #[serde(default)]
    pub dv: i64,
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

/// 签发 token，返回 (token, 有效期秒数)。`device_id` = 登录设备行 id（见 [`Claims::dv`]）。
pub fn sign_token(
    jwt: &Jwt,
    admin_id: i64,
    ver: i32,
    device_id: i64,
    expire_secs: i64,
) -> Result<(String, i64), ApiError> {
    let claims = Claims { sub: admin_id, ver, dv: device_id };
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
    /// 本次请求来自哪个设备行（0 = 老 token / 无设备）。设备列表要标出「当前设备」，
    /// 退出其他设备换发 token 时也要把它带回去。
    pub device_id: i64,
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

        // 设备闸门（项 7）：token 带了设备 id 就得是本人**在用**的设备。
        // 被踢的设备（status=0）在这里断掉，下一次请求 401 —— 与 token_version 的
        // 全局失效互不影响，所以能精确踢一台。
        let device_id = claims.dv;
        if device_id > 0 {
            let dev = AdminLoginDevice::query()
                .filter_eq("id", device_id)
                .map_err(ApiError::from)?
                .one(&state.db)
                .await
                .map_err(ApiError::from)?;
            match dev {
                Some(d) if d.admin_id == admin.id && d.status == 1 => touch_device(state, d).await,
                _ => return Err(ApiError::Unauthorized),
            }
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

        Ok(Auth { admin, roles, perms, is_super, device_id })
    }
}

/// 设备「最后活跃」的写库间隔（分钟）：每个请求都 UPDATE 一次太吵，隔这么久才刷一次。
/// 代价是这一列最多滞后这么久 —— 它是给人看的时间，不是审计时间（审计看 audit_log）。
const ACTIVE_INTERVAL_MINUTES: i64 = 10;

/// 刷新设备最后活跃时间。写失败只打警告：这是展示信息，不该让这次请求 500。
async fn touch_device(state: &AppState, mut d: AdminLoginDevice) {
    let now = crate::util::now();
    if now.signed_duration_since(d.last_active_at) < chrono::Duration::minutes(ACTIVE_INTERVAL_MINUTES) {
        return;
    }
    d.last_active_at = now;
    if let Err(e) = d.update(&state.db).await {
        tracing::warn!("刷新设备活跃时间失败 (id={}): {e}", d.id);
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
            captcha_trajectory: false,
            retain_days: 90,
            job_enabled: true,
            backup_dir: String::new(),
            backup_keep: 7,
            export_retain_days: 7,
            attach_quota_mb: 200,
            security_scan: None,
            hashids_salt: "test-salt".into(),
            hashids_min_len: 8,
            snowflake_worker: 0,
            snowflake_dc: 0,
            encrypt_key: "base64:YmVlLWFkbWluLXRlc3Qta2V5LTMyLWJ5dGVzLW9rISE=".into(),
            apidoc_enabled: false,
            apidoc_password: String::new(),
        }
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let jwt = build_jwt(&cfg()).unwrap();
        let (token, ttl) = sign_token(&jwt, 7, 3, 42, expire_secs(&cfg())).unwrap();
        assert_eq!(ttl, 86400);
        let claims = verify_token(&jwt, &token).unwrap();
        assert_eq!(claims.sub, 7);
        assert_eq!(claims.ver, 3);
        assert_eq!(claims.dv, 42, "设备 id 要随 token 往返");
    }

    /// 老 token（payload 里没有 `dv`）解出来是 0 = 不按设备校验：升级重启不踢人。
    #[test]
    fn legacy_token_without_device_field_decodes_as_zero() {
        let c: Claims = serde_json::from_value(serde_json::json!({ "sub": 7, "ver": 3 })).unwrap();
        assert_eq!(c.dv, 0, "缺 dv 字段 = 0 = 不按设备校验");
    }

    #[test]
    fn tampered_token_rejected() {
        let jwt = build_jwt(&cfg()).unwrap();
        let (token, _) = sign_token(&jwt, 7, 0, 0, expire_secs(&cfg())).unwrap();
        let mut other = cfg();
        other.jwt_secret = "9999999999999999999999999999999999".into();
        assert!(verify_token(&build_jwt(&other).unwrap(), &token).is_err());
        assert!(verify_token(&jwt, "garbage").is_err());
    }
}
