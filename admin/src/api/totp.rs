// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 动态验证码（TOTP）+ 登录设备（项 7）。
//!
//! **自服务接口**：这五条都是「管自己的东西」，不挂权限码 —— 谁登录了都能管自己的
//! 验证码和自己的设备行。密钥落库前用 `crate::crypto` 加密（同 email/phone），
//! 明文只在「生成密钥」那一次回给前端（要扫进验证器 App），之后永不返回、不进日志。
use crate::auth::Auth;
use crate::crypto;
use crate::error::{ApiError, AppJson, BAD_TOTP, TOTP_BOUND, TOTP_NOT_BOUND, ok};
use crate::hid;
use crate::models::{Admin, AdminLoginDevice};
use crate::state::AppState;
use crate::totp;
use crate::util::{now, now_unix};
use axum::Json;
use axum::extract::{Path, State};
use bee_orm::Model;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct CodeBody {
    /// 验证器 App 上的 6 位码
    pub code: String,
}

/// 解出密钥明文（base32）。空串 = 没生成过；解不开报内部错误（存量脏数据）。
pub fn secret_of(state: &AppState, admin: &Admin) -> Result<String, ApiError> {
    crypto::plain(&state.crypto, &admin.totp_secret).map_err(ApiError::internal)
}

/// 校验一枚验证码（登录、绑定、解绑三处共用）。密钥为空恒 false。
pub fn verify_code(state: &AppState, admin: &Admin, code: &str) -> Result<bool, ApiError> {
    let b32 = secret_of(state, admin)?;
    Ok(totp::verify(&totp::base32_decode(&b32), code, now_unix(), totp::WINDOW))
}

#[apidoc::title("生成动态验证码密钥")]
#[apidoc::desc("为当前登录管理员生成一组 TOTP 密钥（base32）并返回二维码，用于验证器 App 绑定。**明文密钥只在这里返回这一次**；重复调用会覆盖上一次还没绑定的密钥。已绑定的要先解绑。需要登录")]
#[apidoc::url("/api/v1/auth/totp/setup")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "object", desc = "密钥与二维码", children = [
            {name = "secret", ty = "string", required, desc = "base32 密钥（32 字符，可手工输入验证器）"},
            {name = "otpauth_url", ty = "string", required, desc = "otpauth:// 标准链接"},
            {name = "qr_svg", ty = "string", required, desc = "二维码 SVG 源码（前端内联展示）"},
        ])]
pub async fn setup(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    if auth.admin.totp_enabled == 1 {
        return Err(ApiError::BadRequest(TOTP_BOUND.into()));
    }
    let b32 = totp::base32_encode(&totp::random_secret());
    let mut admin = auth.admin.clone();
    admin.totp_secret = crypto::write(&state.crypto, &b32).map_err(ApiError::internal)?;
    admin.updated_at = now();
    admin.update(&state.db).await.map_err(ApiError::from)?;

    let url = totp::otpauth_url(&state.cfg.app_name, &admin.username, &b32);
    let svg = totp::qr_svg(&url).map_err(|e| ApiError::internal(format!("生成二维码失败: {e}")))?;
    Ok(ok(json!({ "secret": b32, "otpauth_url": url, "qr_svg": svg })))
}

#[apidoc::title("绑定动态验证码")]
#[apidoc::desc("用验证器 App 上的 6 位码确认密钥可用，确认后登录才要求动态验证码。需要登录")]
#[apidoc::url("/api/v1/auth/totp/enable")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "code", ty = "string", required, desc = "当前 6 位动态验证码", mock = "123456")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn enable(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<CodeBody>,
) -> Result<Json<Value>, ApiError> {
    if auth.admin.totp_enabled == 1 {
        return Err(ApiError::BadRequest(TOTP_BOUND.into()));
    }
    if secret_of(&state, &auth.admin)?.is_empty() {
        return Err(ApiError::BadRequest(TOTP_NOT_BOUND.into()));
    }
    if !verify_code(&state, &auth.admin, &body.code)? {
        return Err(ApiError::BadRequest(BAD_TOTP.into()));
    }
    let mut admin = auth.admin.clone();
    admin.totp_enabled = 1;
    admin.updated_at = now();
    admin.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("解绑动态验证码")]
#[apidoc::desc("校验当前 6 位码后解绑，并把密钥一起清掉（留着等于没解绑）。解绑后登录不再要求动态验证码。需要登录")]
#[apidoc::url("/api/v1/auth/totp/disable")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "code", ty = "string", required, desc = "当前 6 位动态验证码", mock = "123456")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn disable(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<CodeBody>,
) -> Result<Json<Value>, ApiError> {
    if auth.admin.totp_enabled != 1 {
        return Err(ApiError::BadRequest(TOTP_NOT_BOUND.into()));
    }
    if !verify_code(&state, &auth.admin, &body.code)? {
        return Err(ApiError::BadRequest(BAD_TOTP.into()));
    }
    let mut admin = auth.admin.clone();
    admin.totp_enabled = 0;
    admin.totp_secret = String::new();
    admin.updated_at = now();
    admin.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("登录设备列表")]
#[apidoc::desc("当前账号在用（没被踢）的设备，按最后活跃倒序，`current` 标出本次请求来自哪台。需要登录")]
#[apidoc::url("/api/v1/auth/devices")]
#[apidoc::method("GET")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "array", desc = "设备列表", children = [
            {name = "id", ty = "string", required, desc = "设备短串（踢设备时用）"},
            {name = "user_agent", ty = "string", required, desc = "设备指纹（UA 原串）"},
            {name = "ip", ty = "string", desc = "最近一次登录的 IP"},
            {name = "created_at", ty = "string", desc = "首次登录时间"},
            {name = "last_active_at", ty = "string", desc = "最后活跃时间"},
            {name = "current", ty = "bool", required, desc = "是否本次请求所在的设备"},
        ])]
pub async fn devices(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let list = AdminLoginDevice::query()
        .filter_eq("admin_id", auth.admin.id)
        .map_err(ApiError::from)?
        .filter_eq("status", 1)
        .map_err(ApiError::from)?
        .order_by("last_active_at DESC")
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;
    let rows: Vec<Value> = list
        .iter()
        .filter_map(|d| {
            let mut v = serde_json::to_value(d).ok()?;
            v["current"] = json!(d.id == auth.device_id);
            Some(v)
        })
        .collect();
    Ok(ok(json!(rows)))
}

#[apidoc::title("踢出登录设备")]
#[apidoc::desc("把某一台设备踢下线：它手里的 token 下一次请求就 401，其他设备不受影响（与「退出其他设备」的全局失效不同）。只能踢自己的设备。需要登录")]
#[apidoc::url("/api/v1/auth/devices/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "设备短串（列表接口给）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn kick(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = hid::dec(&id)?;
    let found = AdminLoginDevice::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?;
    // 不是自己的设备当不存在（404）：拿 403 等于告诉别人「这个 id 是真的」。
    // 已踢过的行同样 404 —— 列表里也看不到它了，别再给第二条反馈。
    let Some(mut d) = found.filter(|d| d.admin_id == auth.admin.id && d.status == 1) else {
        return Err(ApiError::NotFound);
    };
    d.status = 0;
    d.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}
