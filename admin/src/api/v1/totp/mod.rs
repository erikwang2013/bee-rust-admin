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
use security_rust::throttle::{MemoryThrottleStore, Throttle, ThrottleConfig, ThrottleDecision};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct CodeBody {
    /// 验证器 App 上的 6 位码
    pub code: String,
}

/// 「生成备用码 / 解绑」两条已登录确认端点的当前码失败限流：5 次失败 / 5 分钟 → 429。
/// 成功不清零（窗口自然过期）。进程内 per-admin；多实例不共享，单实例语义一致。
pub fn confirm_throttle() -> Arc<Throttle<MemoryThrottleStore>> {
    Arc::new(Throttle::new(
        MemoryThrottleStore::new(),
        ThrottleConfig {
            threshold: 5,
            window_secs: 300,
            ban_secs: 300,
        },
    ))
}

fn confirm_key(admin_id: i64) -> String {
    format!("totp-confirm:{admin_id}")
}

/// 在验码**之前**检查：额度耗尽后连正确码也要等窗口滑过去。
fn check_confirm(state: &AppState, admin_id: i64) -> Result<(), ApiError> {
    match state.totp_throttle.check(&confirm_key(admin_id), now_unix()) {
        ThrottleDecision::Banned { .. } | ThrottleDecision::Allow { remaining: 0 } => {
            Err(ApiError::throttled(5))
        }
        _ => Ok(()),
    }
}

fn record_confirm_failure(state: &AppState, admin_id: i64) {
    if let Err(e) = state.totp_throttle.record_failure(&confirm_key(admin_id), now_unix()) {
        tracing::error!("TOTP 确认限流计数失败: {e}");
    }
}

/// 解出密钥明文（base32）。空串 = 没生成过；解不开报内部错误（存量脏数据）。
pub fn secret_of(state: &AppState, admin: &Admin) -> Result<String, ApiError> {
    crypto::plain(&state.crypto, &admin.totp_secret).map_err(ApiError::internal)
}

/// 校验一枚验证码（登录、绑定、解绑三处共用）。密钥为空恒 false。
pub fn verify_code(state: &AppState, admin: &Admin, code: &str) -> Result<bool, ApiError> {
    Ok(verify_code_step(state, admin, code)?.is_some())
}

/// 同上，并把命中的步号带出来（S1：登录侧要拿它记账）。
pub fn verify_code_step(state: &AppState, admin: &Admin, code: &str) -> Result<Option<i64>, ApiError> {
    let b32 = secret_of(state, admin)?;
    Ok(totp::verify_step(&totp::base32_decode(&b32), code, now_unix(), totp::WINDOW))
}

/// 登录账本的一次性消费（S1）：`步号 > totp_last_step`（或账本为空）才放行，并在**同一条
/// 语句**里把账本推到该步号 —— 并发登录只有一个赢家，输的那个拿到 `false`。
/// 只有登录走这里：绑定/解绑是已登录会话里的动作，不消费登录账本（但解绑会清空它）。
pub async fn consume_step(state: &AppState, admin_id: i64, step: i64) -> Result<bool, ApiError> {
    let n = state
        .db
        .execute(
            "UPDATE admin SET totp_last_step = ? \
             WHERE id = ? AND (totp_last_step IS NULL OR totp_last_step < ?)",
            &[bee_orm::Value::from(step), bee_orm::Value::from(admin_id), bee_orm::Value::from(step)],
        )
        .await
        .map_err(ApiError::from)?;
    Ok(n > 0)
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
#[apidoc::desc("校验当前 6 位码后解绑，并把密钥与备用码一起清掉（留着等于没解绑）。连续输错 5 次当前码后 5 分钟内一律 429。解绑后登录不再要求动态验证码。需要登录")]
#[apidoc::url("/api/v1/auth/totp/disable")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "code", ty = "string", required, desc = "当前 6 位动态验证码", mock = "123456")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("429")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn disable(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<CodeBody>,
) -> Result<Json<Value>, ApiError> {
    if auth.admin.totp_enabled != 1 {
        return Err(ApiError::BadRequest(TOTP_NOT_BOUND.into()));
    }
    check_confirm(&state, auth.admin.id)?;
    if !verify_code(&state, &auth.admin, &body.code)? {
        record_confirm_failure(&state, auth.admin.id);
        return Err(ApiError::BadRequest(BAD_TOTP.into()));
    }
    // 解绑与删备用码必须同事务（M2）：分开写时删码失败会留下「已解绑但旧备用码还在」，
    // 重试又因未绑定而早退，旧码就永远删不掉了。
    let mut conn = state.db.get().await.map_err(ApiError::from)?;
    conn.begin().await.map_err(ApiError::from)?;
    let result = async {
        // 一次性账本跟着密钥一起清（S1）：留着的话，重新绑定后旧步号的高水位还在，
        // 会把新密钥早期的码误判成重放
        conn.execute(
            "UPDATE admin SET totp_enabled = 0, totp_secret = '', totp_last_step = NULL, \
             updated_at = ? WHERE id = ?",
            &[bee_orm::Value::from(now()), bee_orm::Value::from(auth.admin.id)],
        )
        .await?;
        conn.execute(
            "DELETE FROM admin_backup_code WHERE admin_id = ?",
            &[bee_orm::Value::from(auth.admin.id)],
        )
        .await?;
        Ok::<(), bee_orm::OrmError>(())
    }
    .await;
    match result {
        Ok(()) => conn.commit().await.map_err(ApiError::from)?,
        Err(e) => {
            let _ = conn.rollback().await;
            return Err(e.into());
        }
    }
    Ok(ok(Value::Null))
}

/// 在未使用的备用码里找一枚匹配的（F4-1）。全部行比对完再返回，不提前跳出；
/// 命中返回行 id（调用方拿它做一次性消费）。归一化后为空直接 None。
pub async fn verify_backup_code(
    state: &AppState,
    admin_id: i64,
    code: &str,
) -> Result<Option<i64>, ApiError> {
    let normalized = totp::normalize_backup_code(code);
    if normalized.is_empty() {
        return Ok(None);
    }
    let want = totp::backup_code_hash(&state.cfg.encrypt_key, &normalized);
    let rows = state
        .db
        .query(
            "SELECT id, code_hash FROM admin_backup_code WHERE admin_id = ? AND used_at IS NULL",
            &[bee_orm::Value::from(admin_id)],
        )
        .await
        .map_err(ApiError::from)?;
    let mut hit = None;
    for row in &rows {
        if row.get("code_hash").and_then(|v| v.as_str()) == Some(want.as_str()) {
            hit = row.get("id").and_then(|v| v.as_i64());
        }
    }
    Ok(hit)
}

/// 一次性消费一枚备用码：`used_at IS NULL` 才成立，并发登录只有一个赢家。
pub async fn consume_backup_code(state: &AppState, id: i64) -> Result<bool, ApiError> {
    let n = state
        .db
        .execute(
            "UPDATE admin_backup_code SET used_at = ? WHERE id = ? AND used_at IS NULL",
            &[bee_orm::Value::from(now()), bee_orm::Value::from(id)],
        )
        .await
        .map_err(ApiError::from)?;
    Ok(n > 0)
}

/// 当前账号还剩多少枚未使用的备用码（个人中心展示用）。
pub async fn backup_codes_left(state: &AppState, admin_id: i64) -> Result<i64, ApiError> {
    let rows = state
        .db
        .query(
            "SELECT COUNT(*) AS n FROM admin_backup_code WHERE admin_id = ? AND used_at IS NULL",
            &[bee_orm::Value::from(admin_id)],
        )
        .await
        .map_err(ApiError::from)?;
    Ok(rows.first().and_then(|r| r.get("n")).and_then(|v| v.as_i64()).unwrap_or(0))
}

#[apidoc::title("生成 TOTP 备用码")]
#[apidoc::desc("要求提交当前 6 位动态验证码；一次性生成 10 枚备用码并把旧码全部作废。**明文只在这次响应里出现一次**，库里只存加盐哈希。连续输错 5 次当前码后 5 分钟内一律 429。未绑定动态验证码时 400。需要登录")]
#[apidoc::url("/api/v1/auth/totp/backup-codes")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "code", ty = "string", required, desc = "当前 6 位动态验证码（确认是本人操作）", mock = "123456")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("429")]
#[apidoc::returned(name = "data", ty = "object", desc = "一次性明文备用码", children = [
            {name = "codes", ty = "array", required, desc = "10 枚备用码（每枚 10 位，已剔除 0/O/1/I）"},
        ])]
pub async fn backup_codes(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<CodeBody>,
) -> Result<Json<Value>, ApiError> {
    if auth.admin.totp_enabled != 1 {
        return Err(ApiError::BadRequest(TOTP_NOT_BOUND.into()));
    }
    check_confirm(&state, auth.admin.id)?;
    if !verify_code(&state, &auth.admin, &body.code)? {
        record_confirm_failure(&state, auth.admin.id);
        return Err(ApiError::BadRequest(BAD_TOTP.into()));
    }
    let codes = totp::backup_codes();
    let ids: Vec<i64> = (0..codes.len()).map(|_| state.next_id()).collect::<Result<_, _>>()?;
    let created = now();

    // 删旧 + 插新在同一事务里：生成到一半失败不能把用户仅剩的备用码清空
    let mut conn = state.db.get().await.map_err(ApiError::from)?;
    conn.begin().await.map_err(ApiError::from)?;
    let result = async {
        conn.execute(
            "DELETE FROM admin_backup_code WHERE admin_id = ?",
            &[bee_orm::Value::from(auth.admin.id)],
        )
        .await?;
        for (id, code) in ids.iter().zip(&codes) {
            conn.execute(
                "INSERT INTO admin_backup_code (id, admin_id, code_hash, used_at, created_at) \
                 VALUES (?, ?, ?, NULL, ?)",
                &[
                    bee_orm::Value::from(*id),
                    bee_orm::Value::from(auth.admin.id),
                    bee_orm::Value::from(totp::backup_code_hash(&state.cfg.encrypt_key, code)),
                    bee_orm::Value::from(created),
                ],
            )
            .await?;
        }
        Ok::<(), bee_orm::OrmError>(())
    }
    .await;
    match result {
        Ok(()) => conn.commit().await.map_err(ApiError::from)?,
        Err(e) => {
            let _ = conn.rollback().await;
            return Err(e.into());
        }
    }
    Ok(ok(json!({ "codes": codes })))
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
        // 同一秒活跃的多台设备顺序也要稳定：没有 tie-break 时翻页/刷新可能换序
        .order_by("id DESC")
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
    // 本进程的闸门缓存立即失效：踢出后下一次请求就 401（别的实例最迟 30 秒，
    // 见 auth::forget_device 的注释）
    crate::auth::forget_device(d.admin_id, d.id);
    Ok(ok(Value::Null))
}
