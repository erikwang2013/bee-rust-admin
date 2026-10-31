// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;
use crate::util::{hash_password_async, verify_password_async};
use bee_orm::Value as DbValue;

#[apidoc::title("退出登录")]
#[apidoc::desc("当前设备的 token 立即失效（token_version 自增）。需要登录")]
#[apidoc::url("/api/v1/auth/logout")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn logout(
    State(state): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    // 当前 token 立即失效；设备行一起置 0（否则列表里留一排「在用」的僵尸设备）
    crate::auth::revoke_sessions(&state, auth.admin.id, None, None).await?;
    write_login_log(&state, auth.admin.id, &auth.admin.username, &headers, 1, "退出登录").await;
    Ok(ok(Value::Null))
}

/// 退出其他设备（A3）：token_version 自增让所有旧 token 作废，然后给当前设备
/// 换发一枚新版本的 token —— 用旧版本签的当前设备 token 也一起失效了，
/// 不回新就得把发起者自己也踢下线。前端契约：新 token 在 `data.token`。
#[apidoc::title("退出其他设备")]
#[apidoc::desc("所有旧 token 一起作废，并给当前设备换发新 token（不回新 token 会把发起者自己也踢下线）。需要登录")]
#[apidoc::url("/api/v1/auth/logout-others")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "新 token", children = [
            {name = "token", ty = "string", required, desc = "换发后的 JWT"},
            {name = "expires_in", ty = "int", required, desc = "有效期（秒）"},
        ])]
pub async fn logout_others(
    State(state): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    // 全端下线：版本自增 + 除当前设备外的设备行置 0（当前设备保持 1，换发 token 还挂它）。
    // 本进程的设备闸门缓存也由 helper 一并清掉（别的实例最迟 30 秒）。
    let new_ver =
        crate::auth::revoke_sessions(&state, auth.admin.id, Some(auth.device_id), None).await?;

    // 换发的 token 仍挂在**当前这台设备**上：不然一杯新 token 就把本机从设备列表里摘掉。
    // 老 token（升级前签的，claims 里没有 dv）解出来是 0 —— 原样签回去这台设备就永远
    // 不在列表里（踢不掉），而且循环调用 logout-others 能一直续期。按当前 UA 落一行再签。
    let device_id = if auth.device_id == 0 {
        find_or_create_device(
            &state,
            auth.admin.id,
            &user_agent(&headers),
            &client_ip(&headers),
        )
        .await?
        .id
    } else {
        auth.device_id
    };
    let (token, expires_in) = sign_token(
        &state.jwt,
        auth.admin.id,
        new_ver,
        device_id,
        expire_secs(&state.cfg),
    )?;
    write_login_log(&state, auth.admin.id, &auth.admin.username, &headers, 1, "退出其他设备").await;
    Ok(ok(json!({ "token": token, "expires_in": expires_in })))
}

#[apidoc::title("当前用户信息")]
#[apidoc::desc("个人中心回显：用户资料 + 角色标识 + 权限码集合（email / phone 出口解密为明文）。需要登录")]
#[apidoc::url("/api/v1/auth/profile")]
#[apidoc::method("GET")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "用户 + 角色 + 权限", children = [
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
            {name = "backup_codes_left", ty = "int", desc = "剩余未使用的 TOTP 备用码数（F4）"},
        ]},
            {name = "roles", ty = "array", required, desc = "角色标识数组，如 [\"super\"]"},
            {name = "perms", ty = "array", required, desc = "权限码数组，如 [\"system:admin:list\"]"},
        ])]
pub async fn profile(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let mut perms: Vec<&String> = auth.perms.iter().collect();
    perms.sort();
    let roles: Vec<&str> = auth.roles.iter().map(|r| r.code.as_str()).collect();
    // 库里是密文，回给前端的是明文（个人中心要回显/编辑这几项，B7）
    let email = crypto::plain(&state.crypto, &auth.admin.email).map_err(ApiError::internal)?;
    let phone = crypto::plain(&state.crypto, &auth.admin.phone).map_err(ApiError::internal)?;
    let backup_codes_left = crate::api::totp::backup_codes_left(&state, auth.admin.id).await?;
    Ok(ok(json!({
        "user": {
            "id": hid::enc(auth.admin.id),
            "username": auth.admin.username,
            "nickname": auth.admin.nickname,
            // 列里存的是插件的 saved_path（内部地址），对外一律换成可用的 URL
            "avatar": crate::api::avatar::public_avatar(&auth.admin.avatar, auth.admin.id),
            "email": email,
            "phone": phone,
            "sex": auth.admin.sex,
            "is_super": auth.is_super,
            "dept_id": hid::enc(auth.admin.dept_id),
            // 个人中心要据此显示「绑定 / 解绑动态验证码」（项 7）与备用码剩余数（F4）
            "totp_enabled": auth.admin.totp_enabled == 1,
            "backup_codes_left": backup_codes_left,
        },
        "roles": roles,
        "perms": perms,
    })))
}

/// 当前用户的菜单树（只含目录/菜单；超管全量，否则按角色勾选）。
#[apidoc::title("当前用户菜单树")]
#[apidoc::desc("只含目录 / 菜单（不含按钮）；超管拿全量，其余按已启用角色勾选（缺祖先节点时自动补全）。需要登录")]
#[apidoc::url("/api/v1/auth/menus")]
#[apidoc::method("GET")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "array", desc = "菜单树（顶层节点数组）", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "parent_id", ty = "string", required, desc = "父节点短串（0 = 顶层）"},
            {name = "name", ty = "string", required, desc = "菜单名"},
            {name = "path", ty = "string", desc = "前端路由路径"},
            {name = "icon", ty = "string", desc = "图标名"},
            {name = "children", ty = "array", desc = "子节点（同结构，递归）"},
        ])]
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
                    "id": hid::enc(m.id),
                    "parent_id": hid::enc(m.parent_id),
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

/// 改自己的资料。不碰密码/状态/角色，也不换 token（否则改个昵称就被踢下线）。
#[apidoc::title("修改个人资料")]
#[apidoc::desc("只改自己的昵称 / 邮箱 / 手机号：不碰密码、状态、角色，也不换 token（改个昵称不该被踢下线）")]
#[apidoc::url("/api/v1/auth/profile")]
#[apidoc::method("PUT")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "nickname", ty = "string", desc = "昵称，最长 64 字符")]
#[apidoc::param(name = "email", ty = "string", desc = "邮箱（落库加密），最长 128 字符")]
#[apidoc::param(name = "phone", ty = "string", desc = "手机号（落库加密），最长 20 字符")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn update_profile(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<ProfileBody>,
) -> Result<Json<Value>, ApiError> {
    crate::api::check_len("nickname", &body.nickname, 64)?;
    crate::api::check_len("email", &body.email, 128)?;
    crate::api::check_len("phone", &body.phone, 20)?;

    // 写库一律密文（`crate::crypto::write` 对已是密文的输入原样返回，重放安全）
    let email = crypto::write(&state.crypto, &body.email).map_err(ApiError::BadRequest)?;
    let phone = crypto::write(&state.crypto, &body.phone).map_err(ApiError::BadRequest)?;
    // 只写这三列：整行 update 会把读快照里的 password / status / token_version 写回去，
    // 撤销并发的改密或禁用
    state
        .db
        .execute(
            "UPDATE admin SET nickname = ?, email = ?, phone = ?, updated_at = ? WHERE id = ?",
            &[
                DbValue::from(body.nickname),
                DbValue::from(email),
                DbValue::from(phone),
                DbValue::from(now()),
                DbValue::from(auth.admin.id),
            ],
        )
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("修改自己的密码")]
#[apidoc::desc("校验原密码后换成新密码（argon2id）；成功后所有端下线（token_version 自增），需重新登录")]
#[apidoc::url("/api/v1/auth/password")]
#[apidoc::method("PUT")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "old_password", ty = "string", required, desc = "原密码")]
#[apidoc::param(name = "new_password", ty = "string", required, desc = "新密码，至少 8 位且含字母/数字/符号中至少两类")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn change_password(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<ChangePasswordBody>,
) -> Result<Json<Value>, ApiError> {
    // 超长的原密码不可能与库里的密码匹配：在 argon2 之前截停，省掉一次无谓的 KDF。
    // 与普通「原密码错误」同一句话，不暴露是长度问题。
    if body.old_password.chars().count() > crate::api::admin::PASSWORD_MAX {
        return Err(ApiError::BadRequest("原密码错误".into()));
    }
    crate::api::admin::validate_password(&body.new_password, "新密码")?;
    if !verify_password_async(body.old_password.clone(), auth.admin.password.clone()).await {
        return Err(ApiError::BadRequest("原密码错误".into()));
    }
    // 只写 password / token_version / updated_at，并把设备行置 0：
    // 全端下线，需重新登录（设备缓存由 helper 一并清掉）
    let password = hash_password_async(body.new_password).await.map_err(ApiError::internal)?;
    crate::auth::revoke_sessions(&state, auth.admin.id, None, Some(&password)).await?;
    Ok(ok(Value::Null))
}
