// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::auth::{Auth, sign_token};
use crate::config::AppConfig;
use crate::error::{ApiError, AppJson, AppPath, ok};
use crate::models::{Admin, LoginLog, Menu};
use crate::state::AppState;
use crate::util::{hash_password, now, now_unix, verify_password};
use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use bee_orm::Model;
use security_rust::throttle::{MemoryThrottleStore, Throttle, ThrottleConfig, ThrottleDecision};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use crate::relations::RelationsExt;

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

/// 用户名不存在时也要付一次 argon2 校验，抹平「用户存在与否」的响应时差
/// （A2：提前返回会跳过 argon2，快出来的那截就是用户名枚举的时序侧信道）。
/// 这是用本项目 `util::hash_password("bee-admin-dummy")` 生成的一次性 argon2id
/// 哈希，只烧 CPU，结果丢弃。
const DUMMY_PASSWORD_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$dhEKLDVWOgNaVRGlknZrTw$SoG1K6HuZZmwffQCWSyxO55B9KpBGRKrsxC8w+CVhTs";

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
    admin_id: i64,
    username: &str,
    headers: &HeaderMap,
    status: i8,
    msg: &str,
) {
    let log = LoginLog {
        id: 0,
        admin_id,
        username: username.chars().take(64).collect(),
        ip: client_ip(headers),
        user_agent: user_agent(headers),
        status,
        msg: msg.chars().take(255).collect(),
        created_at: now(),
    };
    if let Err(e) = log.insert(&state.db).await {
        // 登录记录写失败不能影响登录本身
        tracing::error!("写登录记录失败: {e}");
    }
}

/// 登录限流闸门：`max_fail` 次窗口内失败即封 `lock_minutes` 分钟，账号与 IP 同一阈值。
/// 两个维度同阈值是有意的：IP 桶比账号桶宽松时，攻击者能用「不触发自己上限」的
/// 失败量反复把别人的账号刷封，而自己不被拦。
/// `max_fail <= 0` 或 `lock_minutes <= 0` = 关闭（沿用既有配置约定）。
pub fn login_throttle(cfg: &AppConfig) -> Option<Arc<Throttle<MemoryThrottleStore>>> {
    if cfg.max_fail <= 0 || cfg.lock_minutes <= 0 {
        return None;
    }
    // 饱和乘法自带上限，不必再 clamp 一次；throttle 收 u64 秒
    // （`lock_minutes > 0` 已由上面的早返回保证，i64 → u64 无损）
    let secs = cfg.lock_minutes.saturating_mul(60) as u64;
    Some(Arc::new(Throttle::new(
        MemoryThrottleStore::new(),
        ThrottleConfig {
            threshold: cfg.max_fail.min(u32::MAX as i64) as u32,
            window_secs: secs,
            ban_secs: secs,
        },
    )))
}

/// 账号维度的桶名。截断到与 `login_log.username` 同样的 64 字符：
/// 库要求 key 有界（否则人人换一个超长用户名就能撑爆内存）。
fn user_key(username: &str) -> String {
    format!("user:{}", username.chars().take(64).collect::<String>())
}

/// IP 维度的桶名。无 X-Real-IP / X-Forwarded-For（直连）时 `client_ip()` 返回
/// "unknown"：所有人会挤进同一个桶，按它限流等于误伤全体用户 → 这一维直接不做。
fn ip_key(ip: &str) -> Option<String> {
    (ip != "unknown").then(|| format!("ip:{ip}"))
}

/// 一次登录失败计两个维度：账号（谁被猜）+ IP（从哪猜的）。
/// 计数写失败不影响响应 —— 限流是纵深防御，不是主认证闸门。
fn record_login_failure(state: &AppState, username: &str, ip: &str) {
    let Some(throttle) = &state.throttle else { return };
    let now = now_unix();
    // 达到阈值的判定由库在 record_failure 内部做（返回 Banned），这里不用再算
    let mut keys = vec![user_key(username)];
    keys.extend(ip_key(ip));
    for key in keys {
        if let Err(e) = throttle.record_failure(&key, now) {
            tracing::error!("登录限流计数失败 ({key}): {e}");
        }
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
    let username = body.username.trim();
    let ip = client_ip(&headers);

    // 限流闸门在验密之前：过不去就不比对密码，响应也不泄露密码是否正确
    if let Some(throttle) = &state.throttle {
        let now = now_unix();
        let uk = user_key(username);
        let ik = ip_key(&ip);
        let keys: Vec<&str> = std::iter::once(uk.as_str()).chain(ik.as_deref()).collect();
        let decision = throttle.check_any(&keys, now);
        // `Allow { remaining: 0 }` 是「额度已耗尽、本请求应被拒绝」，不是「还能再试一次」
        if matches!(
            decision,
            ThrottleDecision::Banned { .. } | ThrottleDecision::Allow { remaining: 0 }
        ) {
            write_login_log(&state, 0, username, &headers, 0, "请求过于频繁").await;
            // 封禁时长 = 窗口 = lock_minutes，额度耗尽最晚也是等窗口滑完
            return Err(ApiError::throttled(state.cfg.lock_minutes));
        }
        // 存储故障不 fail-closed（库有意如此）：限流是纵深防御，主认证闸门在后面
        if decision == ThrottleDecision::Unavailable {
            tracing::warn!("登录限流存储不可用，本次放行");
        }
    }

    let admin = Admin::query()
        .filter_eq("username", username)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?;

    let Some(admin) = admin else {
        // 与下面「密码错误」分支付同样的 argon2 代价（结果丢弃），
        // 否则两条路径的耗时差就是用户名枚举的侧信道
        let _ = verify_password(&body.password, DUMMY_PASSWORD_HASH);
        write_login_log(&state, 0, username, &headers, 0, "用户不存在").await;
        record_login_failure(&state, username, &ip);
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    };
    if !verify_password(&body.password, &admin.password) {
        write_login_log(&state, admin.id, &admin.username, &headers, 0, "密码错误").await;
        record_login_failure(&state, username, &ip);
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    }
    if admin.status != 1 {
        write_login_log(&state, admin.id, &admin.username, &headers, 0, "账号已禁用").await;
        record_login_failure(&state, username, &ip);
        return Err(ApiError::BadRequest("账号已被禁用".into()));
    }

    let (token, expires_in) = sign_token(admin.id, admin.token_version, &state.cfg)?;

    let mut updated = admin.clone();
    updated.last_login_at = Some(now());
    updated.last_login_ip = client_ip(&headers);
    updated.updated_at = now();
    updated.update(&state.db).await.map_err(ApiError::from)?;

    // 成功即清掉该账号的失败计数（修复①：4 次失败 + 1 次成功 + 1 次失败不该锁死）。
    // IP 桶**不清**：桶是共享的，攻击者拿自己账号登录一次就能替爆破者洗掉计数，
    // 封禁也会被顺手解除 —— 库的 record_success 只清计数不清封禁，正是为此。
    if let Some(throttle) = &state.throttle {
        if let Err(e) = throttle.record_success(&user_key(username)) {
            tracing::error!("登录成功后清零限流计数失败: {e}");
        }
    }

    write_login_log(&state, admin.id, &admin.username, &headers, 1, "登录成功").await;

    Ok(ok(json!({
        "token": token,
        "expires_in": expires_in,
        "user": {
            "id": admin.id,
            "username": admin.username,
            "nickname": admin.nickname,
            "avatar": admin.avatar,
            "email": admin.email,
            "phone": admin.phone,
            "sex": admin.sex,
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
    admin.update(&state.db).await.map_err(ApiError::from)?;
    write_login_log(&state, admin.id, &admin.username, &headers, 1, "退出登录").await;
    Ok(ok(Value::Null))
}

/// 退出其他设备（A3）：token_version 自增让所有旧 token 作废，然后给当前设备
/// 换发一枚新版本的 token —— 用旧版本签的当前设备 token 也一起失效了，
/// 不回新就得把发起者自己也踢下线。前端契约：新 token 在 `data.token`。
pub async fn logout_others(
    State(state): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let mut admin = auth.admin.clone();
    admin.token_version += 1;
    admin.updated_at = now();
    admin.update(&state.db).await.map_err(ApiError::from)?;

    let (token, expires_in) = sign_token(admin.id, admin.token_version, &state.cfg)?;
    write_login_log(&state, admin.id, &admin.username, &headers, 1, "退出其他设备").await;
    Ok(ok(json!({ "token": token, "expires_in": expires_in })))
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
            // 个人中心要回显/编辑这三项（B7）
            "email": auth.admin.email,
            "phone": auth.admin.phone,
            "sex": auth.admin.sex,
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
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;

    let allowed: Option<Vec<i64>> = if auth.is_super {
        None
    } else {
        let mut ids: Vec<i64> = Vec::new();
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
        let parent_of: HashMap<i64, i64> = all.iter().map(|m| (m.id, m.parent_id)).collect();
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

    fn build(parent: i64, nodes: &[&Menu]) -> Vec<Value> {
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

#[derive(Deserialize)]
pub struct ProfileBody {
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
}

#[derive(Deserialize)]
pub struct AvatarBody {
    /// `data:image/png;base64,…`
    pub data_url: String,
}

/// 改自己的资料。不碰密码/状态/角色，也不换 token（否则改个昵称就被踢下线）。
pub async fn update_profile(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<ProfileBody>,
) -> Result<Json<Value>, ApiError> {
    crate::api::check_len("nickname", &body.nickname, 64)?;
    crate::api::check_len("email", &body.email, 128)?;
    crate::api::check_len("phone", &body.phone, 20)?;

    let mut admin = auth.admin.clone();
    admin.nickname = body.nickname;
    admin.email = body.email;
    admin.phone = body.phone;
    admin.updated_at = now();
    admin.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

/// 头像解码后的大小上限（前端会先压缩；这是服务端的硬闸）。
const AVATAR_MAX_BYTES: usize = 512 * 1024;

/// 解 data URL →（扩展名, 字节）。只认 PNG/JPEG，并用魔数复核，不信客户端的声明。
fn decode_avatar(data_url: &str) -> Result<(&'static str, Vec<u8>), ApiError> {
    let (mime, b64) = data_url
        .split_once(',')
        .ok_or_else(|| ApiError::BadRequest("头像数据格式错误".into()))?;
    let (ext, magic): (&str, &[u8]) = match mime {
        "data:image/png;base64" => ("png", &[0x89, b'P', b'N', b'G']),
        "data:image/jpeg;base64" => ("jpg", &[0xFF, 0xD8, 0xFF]),
        _ => return Err(ApiError::BadRequest("头像仅支持 PNG/JPEG".into())),
    };
    // base64 先按长度卡一道，别为超限数据白解码（4/3 膨胀 + padding）
    if b64.len() > AVATAR_MAX_BYTES / 3 * 4 + 4 {
        return Err(ApiError::BadRequest("头像不能超过 512 KB".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| ApiError::BadRequest("头像 base64 解码失败".into()))?;
    if bytes.len() > AVATAR_MAX_BYTES {
        return Err(ApiError::BadRequest("头像不能超过 512 KB".into()));
    }
    if !bytes.starts_with(magic) {
        return Err(ApiError::BadRequest("文件内容不是有效的图片".into()));
    }
    Ok((ext, bytes))
}

fn avatar_path(cfg: &crate::config::AppConfig, id: i64, ext: &str) -> std::path::PathBuf {
    std::path::Path::new(&cfg.upload_dir).join("avatar").join(format!("{id}.{ext}"))
}

pub async fn upload_avatar(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<AvatarBody>,
) -> Result<Json<Value>, ApiError> {
    let (ext, bytes) = decode_avatar(&body.data_url)?;
    let dir = avatar_path(&state.cfg, auth.admin.id, ext);
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ApiError::internal(format!("创建头像目录失败: {e}")))?;
    }
    // 换格式时清掉旧文件，同一个 id 只留一份
    for other in ["png", "jpg"] {
        if other != ext {
            let _ = std::fs::remove_file(avatar_path(&state.cfg, auth.admin.id, other));
        }
    }
    std::fs::write(&dir, &bytes).map_err(|e| ApiError::internal(format!("写头像文件失败: {e}")))?;

    let mut admin = auth.admin.clone();
    admin.avatar = avatar_url(admin.id);
    admin.updated_at = now();
    admin.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(json!({ "avatar": admin.avatar })))
}

fn avatar_url(id: i64) -> String {
    format!("/api/v1/avatar/{id}")
}

/// 读头像：公开接口（`<img>` 带不了 Authorization 头），按 id + 扩展名定位文件。
/// 路径参数是 i64，不存在路径穿越。
pub async fn get_avatar(
    State(state): State<AppState>,
    AppPath(id): AppPath<i64>,
) -> Result<Response, ApiError> {
    for (ext, ct) in [("png", "image/png"), ("jpg", "image/jpeg")] {
        if let Ok(bytes) = std::fs::read(avatar_path(&state.cfg, id, ext)) {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(axum::http::header::CONTENT_TYPE, ct.parse().expect("静态 MIME"));
            // no-cache：仍可缓存但每次回源校验，换头像后立刻生效
            headers.insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-cache"),
            );
            return Ok((headers, bytes).into_response());
        }
    }
    Err(ApiError::NotFound)
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
    admin.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1x1 PNG（真实文件字节）。
    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    #[test]
    fn avatar_decoding_rules() {
        let (ext, bytes) = decode_avatar(&format!("data:image/png;base64,{PNG_B64}")).unwrap();
        assert_eq!(ext, "png");
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));

        // 声明 JPEG 实为 PNG：魔数复核不通过（不信客户端的 MIME）
        assert!(decode_avatar(&format!("data:image/jpeg;base64,{PNG_B64}")).is_err());
        // 不支持的 MIME / 没有逗号 / 坏 base64 / 空数据
        assert!(decode_avatar("data:image/gif;base64,R0lGOD").is_err());
        assert!(decode_avatar("nonsense").is_err());
        assert!(decode_avatar("data:image/png;base64,!!!!").is_err());
        assert!(decode_avatar("data:image/png;base64,").is_err());
    }

    #[test]
    fn avatar_rejects_oversize() {
        // 700 KB base64 ≈ 525 KB 解码后 > 512 KB，长度闸在解码前就拦下
        let huge = format!("data:image/png;base64,{}", "A".repeat(700 * 1024));
        assert!(decode_avatar(&huge).is_err());
    }

    /// 假哈希必须是**可解析**的 argon2 串：解析失败时 `verify_password` 会立刻返回
    /// false（不跑 KDF），A2 的抹平效果就静默失效了。
    #[test]
    fn dummy_hash_is_a_real_argon2_hash() {
        let h = argon2::PasswordHash::new(DUMMY_PASSWORD_HASH)
            .expect("占位哈希必须是合法 argon2id PHC 串");
        assert_eq!(h.algorithm.as_str(), "argon2id");
        // 且真的会跑一遍校验（不 panic、不提前返回）
        assert!(!verify_password("not-the-dummy-plaintext", DUMMY_PASSWORD_HASH));
    }

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
