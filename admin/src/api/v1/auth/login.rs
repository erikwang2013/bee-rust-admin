// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;
use crate::util::verify_password_async;
use bee_orm::Value as DbValue;

#[apidoc::title("登录")]
#[apidoc::desc("用户名 + 密码登录，返回 JWT 与当前用户信息。受失败锁定（[auth] max_fail / lock_minutes）与图形验证码（[auth] captcha）保护：过不去就不比密码，响应不泄露密码是否正确")]
#[apidoc::url("/api/v1/auth/login")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::param(name = "username", ty = "string", required, desc = "用户名", mock = "admin")]
#[apidoc::param(name = "password", ty = "string", required, desc = "密码（明文提交，务必走 HTTPS）", mock = "admin123")]
#[apidoc::param(name = "captcha_key", ty = "string", desc = "[auth] captcha = true 时必填：/api/v1/captcha/new 返回的 key")]
#[apidoc::param(name = "captcha_answer", ty = "object", desc = "验证码答案，形状同 poster-rust Answer（Slider 滑动 / Rotate 旋转 / Click 点选）")]
#[apidoc::param(name = "code", ty = "string", desc = "动态验证码或备用码（该账号绑定 TOTP 后必填；6 位纯数字按 TOTP 校验，其余按备用码校验；缺了回 err = auth.totp_required）", mock = "123456")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("429")]
#[apidoc::returned(name = "data", ty = "object", desc = "登录结果", children = [
            {name = "token", ty = "string", required, desc = "JWT，后续请求放 Authorization: Bearer <token>"},
            {name = "expires_in", ty = "int", required, desc = "有效期（秒）"},
            {name = "user", ty = "object", required, desc = "当前用户", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "username", ty = "string", required, desc = "用户名"},
            {name = "nickname", ty = "string", desc = "昵称"},
            {name = "avatar", ty = "string", desc = "头像地址（无头像为空串）"},
            {name = "email", ty = "string", desc = "邮箱（库里密文，出口明文）"},
            {name = "phone", ty = "string", desc = "手机号（同上）"},
            {name = "sex", ty = "int", desc = "性别：0 未知 / 1 男 / 2 女"},
            {name = "is_super", ty = "bool", desc = "是否超级管理员"},
            {name = "dept_id", ty = "string", desc = "部门短串（0 = 无部门）"},
            {name = "totp_enabled", ty = "bool", desc = "是否已绑定动态验证码（项 7）"},
        ]},
        ])]
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    AppJson(body): AppJson<LoginBody>,
) -> Result<Json<Value>, ApiError> {
    if body.username.trim().is_empty() || body.password.is_empty() {
        return Err(ApiError::BadRequest("用户名和密码不能为空".into()));
    }
    let username = body.username.trim();
    // body 上限 8 MiB：超长的用户名 / 密码必须在这里截停，不能带进限流、查库或
    // argon2（匿名请求拿超长密码就能白烧一次 KDF）。用户名创建时最多 64 字符，
    // 密码上限 1024 字符（兼容存量账号）。与其它登录失败同一句话，不暴露是哪一项超限。
    if username.chars().count() > LOGIN_USERNAME_MAX
        || body.password.chars().count() > crate::api::admin::PASSWORD_MAX
    {
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    }
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

    // 验证码闸门（[auth] captcha）放在限流之后、查库之前：
    // - 限流之后：封禁中的请求仍是 429（既有行为不变），验证码不能变成绕过封禁的路子；
    // - 查库之前：没过验证码就不碰库、不烧 argon2，也就没有用户名侧信道可用。
    if state.cfg.captcha {
        let identity = client_ip(&headers);
        if let Err(e) = verify_captcha(
            &state.captcha,
            body.captcha_key.as_deref(),
            body.captcha_answer.clone(),
            &identity,
        ) {
            write_login_log(&state, 0, username, &headers, 0, CAPTCHA_FAILED).await;
            return Err(e);
        }
    }

    // 用户名只能是 ASCII 字母 / 数字 / 下划线（与创建时同一字符集）。MySQL 的
    // utf8mb4_0900_ai_ci 大小写 / 重音不敏感：`Admin` / `ádmin` 都能命中 `admin`，
    // 而限流桶按原始串分 —— 重音变体每次换一个新桶就绕过了失败锁定。这类串在库里
    // 不可能存在，按「用户不存在」处理（同样烧一次 argon2），不查库。
    if !valid_login_username(username) {
        let _ = verify_password_async(body.password.clone(), DUMMY_PASSWORD_HASH.to_string()).await;
        write_login_log(&state, 0, username, &headers, 0, "用户不存在").await;
        record_login_failure(&state, username, &ip);
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
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
        let _ = verify_password_async(body.password.clone(), DUMMY_PASSWORD_HASH.to_string()).await;
        write_login_log(&state, 0, username, &headers, 0, "用户不存在").await;
        record_login_failure(&state, username, &ip);
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    };
    if !verify_password_async(body.password.clone(), admin.password.clone()).await {
        write_login_log(&state, admin.id, &admin.username, &headers, 0, "密码错误").await;
        record_login_failure(&state, username, &ip);
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    }
    if admin.status != 1 {
        write_login_log(&state, admin.id, &admin.username, &headers, 0, "账号已禁用").await;
        record_login_failure(&state, username, &ip);
        return Err(ApiError::BadRequest("账号已被禁用".into()));
    }

    // 动态验证码闸门（项 7）：绑定了才要。放在密码之后 —— 码是第二因子，
    // 先要码等于让不知道密码的人也能把这条路径走一遍。
    // S1：一枚码只换一次登录（同一 30 秒窗里的第二台设备得等下一枚）。
    if admin.totp_enabled == 1 {
        let code = body.code.as_deref().unwrap_or("");
        if code.trim().is_empty() {
            // 客户端没带码（老前端/脚本）是协议没对齐，不是有人在猜密码：
            // 留档但**不计数** —— 否则一个没升级的前端能把用户刷进封禁。
            write_login_log(&state, admin.id, &admin.username, &headers, 0, "需要动态验证码").await;
            return Err(ApiError::BadRequest(crate::error::TOTP_REQUIRED.into()));
        }
        // 分流（F4-1）：6 位纯数字 = TOTP；其余一律按备用码（10 位）验。
        if crate::totp::is_totp_code(code) {
            match crate::api::totp::verify_code_step(&state, &admin, code)? {
                None => {
                    write_login_log(&state, admin.id, &admin.username, &headers, 0, "动态验证码错误").await;
                    record_login_failure(&state, username, &ip);
                    return Err(ApiError::BadRequest(crate::error::BAD_TOTP.into()));
                }
                Some(step) => {
                    // 记账与放行是同一条 UPDATE（`步号 > last_step` 才成立），并发登录只有一个
                    // 赢家。被拒的重放不许碰 last_login_at，也不许落设备行/发 token。
                    if !crate::api::totp::consume_step(&state, admin.id, step).await? {
                        write_login_log(&state, admin.id, &admin.username, &headers, 0, "动态验证码已使用").await;
                        record_login_failure(&state, username, &ip);
                        return Err(ApiError::BadRequest(crate::error::TOTP_REPLAYED.into()));
                    }
                }
            }
        } else {
            // 备用码：未命中与「并发里被别人先销掉」都按错码处理（同一套计数/错误码）
            let hit = crate::api::totp::verify_backup_code(&state, admin.id, code).await?;
            match hit {
                Some(id) if crate::api::totp::consume_backup_code(&state, id).await? => {}
                _ => {
                    let msg = if hit.is_some() { "备用码已使用" } else { "备用码错误" };
                    write_login_log(&state, admin.id, &admin.username, &headers, 0, msg).await;
                    record_login_failure(&state, username, &ip);
                    return Err(ApiError::BadRequest(crate::error::BAD_TOTP.into()));
                }
            }
        }
    }

    // 登录成功落库：只写登录时间 / IP，不再整行写回。WHERE 带 status 与 token_version
    // 的读时快照 —— 校验密码 / TOTP 期间账号可能已被禁用或改密，整行 update 会把那次
    // 变更抹掉（禁用被撤销、被重置的密码被换回旧值），并签发一枚旧版本的 token。
    // totp_last_step 已由上面的 consume_step CAS 写过，这里不碰。
    let ts = now();
    let affected = state
        .db
        .execute(
            "UPDATE admin SET last_login_at = ?, last_login_ip = ?, updated_at = ? \
             WHERE id = ? AND status = 1 AND token_version = ?",
            &[
                DbValue::from(ts),
                DbValue::from(ip.clone()),
                DbValue::from(ts),
                DbValue::from(admin.id),
                DbValue::from(admin.token_version),
            ],
        )
        .await
        .map_err(ApiError::from)?;
    if affected == 0 {
        // affected_rows 只数「真正变了」的行：DATETIME 是秒精度，同一秒内重复登录
        // 写回的值完全相同，这里会是 0，不能直接当成并发变更（正常登录会被误杀）。
        // 按同一快照条件再确认一次：行还在 = 值没变，照常发 token；行没了才是真的
        // 被禁用 / 改密。
        let still_valid = Admin::query()
            .filter_eq("id", admin.id)
            .map_err(ApiError::from)?
            .filter_eq("status", 1)
            .map_err(ApiError::from)?
            .filter_eq("token_version", admin.token_version)
            .map_err(ApiError::from)?
            .count(&state.db)
            .await
            .map_err(ApiError::from)?
            > 0;
        if !still_valid {
            // 快照期间状态/版本变了：不发 token（静默返回会让人以为是自己密码错了）
            write_login_log(&state, admin.id, &admin.username, &headers, 0, "账号状态已变更").await;
            return Err(ApiError::Unauthorized);
        }
    }

    // 设备行（项 7）：同账号同 UA 复用一条，token 里带上它的 id，之后能单独踢掉这台
    let device = find_or_create_device(&state, admin.id, &user_agent(&headers), &ip).await?;
    let (token, expires_in) = sign_token(
        &state.jwt,
        admin.id,
        admin.token_version,
        device.id,
        expire_secs(&state.cfg),
    )?;

    // 成功即清掉该账号的失败计数（修复①：4 次失败 + 1 次成功 + 1 次失败不该锁死）。
    // IP 桶**不清**：桶是共享的，攻击者拿自己账号登录一次就能替爆破者洗掉计数，
    // 封禁也会被顺手解除 —— 库的 record_success 只清计数不清封禁，正是为此。
    if let Some(throttle) = &state.throttle {
        if let Err(e) = throttle.record_success(&user_key(username)) {
            tracing::error!("登录成功后清零限流计数失败: {e}");
        }
    }

    write_login_log(&state, admin.id, &admin.username, &headers, 1, "登录成功").await;

    // 库里 email / phone 是密文，出去前解成明文（前端无感）
    let email = crypto::plain(&state.crypto, &admin.email).map_err(ApiError::internal)?;
    let phone = crypto::plain(&state.crypto, &admin.phone).map_err(ApiError::internal)?;
    // avatar 列存的是插件的 saved_path（内部地址），与 profile 一样换成可用的 URL
    let avatar = crate::api::avatar::public_avatar(&admin.avatar, admin.id);

    Ok(ok(json!({
        "token": token,
        "expires_in": expires_in,
        "user": {
            "id": hid::enc(admin.id),
            "username": admin.username,
            "nickname": admin.nickname,
            "avatar": avatar,
            "email": email,
            "phone": phone,
            "sex": admin.sex,
            "is_super": admin.is_super == 1,
            "dept_id": hid::enc(admin.dept_id),
            "totp_enabled": admin.totp_enabled == 1,
        }
    })))
}
