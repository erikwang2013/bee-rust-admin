// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;
use crate::util::hash_password_async;
use bee_orm::Value as DbValue;

#[apidoc::title("管理员详情")]
#[apidoc::desc("编辑弹窗回显：管理员字段 + role_ids（email / phone 明文），目标超出数据范围时 403")]
#[apidoc::url("/api/v1/admins/{id}")]
#[apidoc::method("GET")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
#[apidoc::returned(name = "data", ty = "object", desc = "管理员对象", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "username", ty = "string", required, desc = "用户名"},
            {name = "nickname", ty = "string", desc = "昵称"},
            {name = "avatar", ty = "string", desc = "头像地址（无头像为空串）"},
            {name = "email", ty = "string", desc = "邮箱（库里密文，出口明文）"},
            {name = "phone", ty = "string", desc = "手机号（同上）"},
            {name = "sex", ty = "int", desc = "性别：0 未知 / 1 男 / 2 女"},
            {name = "dept_id", ty = "string", desc = "部门短串（0 = 无部门）"},
            {name = "dept_name", ty = "string", desc = "部门名（列表补的）"},
            {name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用"},
            {name = "is_super", ty = "bool", desc = "是否超级管理员"},
            {name = "remark", ty = "string", desc = "备注"},
            {name = "last_login_at", ty = "string", desc = "最后登录时间（空 = 从未登录）"},
            {name = "last_login_ip", ty = "string", desc = "最后登录 IP"},
            {name = "created_at", ty = "string", desc = "创建时间"},
            {name = "updated_at", ty = "string", desc = "更新时间"},
            {name = "role_ids", ty = "array", desc = "角色短串数组"},
            {name = "role_names", ty = "array", desc = "角色名数组"},
        ])]
pub async fn detail(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:list")?;
    let id = hid::dec(&id)?;
    let a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    datascope::ensure_admin_in_scope(&auth, &state.db, &a).await?;
    let role_ids = state
        .db
        .get_relations("admin_role", ("admin_id", id), "role_id")
        .await
        .map_err(ApiError::from)?;
    let mut v = serde_json::to_value(&a).map_err(|e| ApiError::internal(format!("序列化失败: {e}")))?;
    // email / phone 落库是密文，详情/编辑回显要明文（前端无感）
    crate::crypto::plain_json(&state.crypto, &mut v)?;
    // 同列表：avatar 列存的是 saved_path，回给前端的是 URL
    v["avatar"] = json!(crate::api::avatar::public_avatar(&a.avatar, a.id));
    v["role_ids"] = json!(hid::enc_vec(&role_ids));
    v["is_super"] = json!(a.is_super == 1); // 前端契约：bool
    Ok(ok(v))
}

/// 用户名重复 → 该接口的专属文案（`admin.username_taken`）。
/// 唯一键冲突的识别在 `ApiError::from` 里按 MySQL 报错文本做（上游 `OrmError`
/// 没有独立变体），冲突是它唯一映射成 `BadRequest` 的情况，其余是 `NotFound` / `Internal`。
pub(crate) fn username_taken(e: OrmError) -> ApiError {
    match ApiError::from(e) {
        ApiError::BadRequest(_) => ApiError::BadRequest("用户名已存在".into()),
        other => other,
    }
}

#[apidoc::title("新建管理员")]
#[apidoc::desc("用户名 3-64 字符（字母/数字/下划线）；部门与角色都不能超出操作者的数据范围")]
#[apidoc::url("/api/v1/admins")]
#[apidoc::method("POST")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "username", ty = "string", required, desc = "用户名，3-64 个字符（字母 / 数字 / 下划线，唯一）")]
#[apidoc::param(name = "password", ty = "string", required, desc = "登录密码，至少 8 位且含字母/数字/符号中至少两类")]
#[apidoc::param(name = "nickname", ty = "string", desc = "昵称，最长 64 字符")]
#[apidoc::param(name = "email", ty = "string", desc = "邮箱（落库加密），最长 128 字符")]
#[apidoc::param(name = "phone", ty = "string", desc = "手机号（落库加密），最长 20 字符")]
#[apidoc::param(name = "sex", ty = "int", desc = "性别：0 未知 / 1 男 / 2 女")]
#[apidoc::param(name = "dept_id", ty = "string", desc = "部门短串（0 = 无部门）")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注，最长 255 字符")]
#[apidoc::param(name = "role_ids", ty = "array", desc = "角色短串数组")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "object", desc = "新建结果", children = [
            {name = "id", ty = "string", required, desc = "新管理员对外 id（hashids 短串）"},
        ])]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<AdminCreateBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:add")?;
    validate_username(&body.username)?;
    validate_password(&body.password, "密码")?;
    validate_profile(&body.nickname, &body.email, &body.phone, &body.remark)?;
    // 建人也要在范围内：部门与角色都不能超出操作者自己（B5）
    datascope::ensure_dept_in_scope(&auth, &state.db, body.dept_id).await?;
    datascope::ensure_roles_grantable(&auth, &state.db, &body.role_ids).await?;

    let password = hash_password_async(body.password).await.map_err(ApiError::internal)?;
    let a = Admin {
        id: state.next_id()?,
        username: body.username.trim().to_string(),
        password,
        nickname: body.nickname,
        email: sealed(&state.crypto, &body.email)?,
        phone: sealed(&state.crypto, &body.phone)?,
        sex: body.sex,
        avatar: String::new(),
        dept_id: body.dept_id,
        status: body.status,
        is_super: 0,
        totp_secret: String::new(),
        totp_enabled: 0,
        totp_last_step: None,
        token_version: 0,
        last_login_at: None,
        last_login_ip: String::new(),
        remark: body.remark,
        created_at: now(),
        updated_at: now(),
    };
    // id 已在上面发号，insert 就够（create 的读回是给自增主键用的）
    a.insert(&state.db).await.map_err(username_taken)?;
    state
        .db
        .set_relations("admin_role", ("admin_id", a.id), "role_id", &dedup_ids(body.role_ids))
        .await
        .map_err(ApiError::from)?;

    Ok(ok(json!({ "id": hid::enc(a.id) })))
}

#[apidoc::title("编辑管理员")]
#[apidoc::desc("本接口同样能改状态：禁用即踢下线；不能改自己 / 超管的状态，目标与要授的角色都不能超出操作者范围")]
#[apidoc::url("/api/v1/admins/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::param(name = "nickname", ty = "string", desc = "昵称，最长 64 字符")]
#[apidoc::param(name = "email", ty = "string", desc = "邮箱（落库加密），最长 128 字符")]
#[apidoc::param(name = "phone", ty = "string", desc = "手机号（落库加密），最长 20 字符")]
#[apidoc::param(name = "sex", ty = "int", desc = "性别：0 未知 / 1 男 / 2 女")]
#[apidoc::param(name = "dept_id", ty = "string", desc = "部门短串（0 = 无部门）")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用")]
#[apidoc::param(name = "remark", ty = "string", desc = "备注，最长 255 字符")]
#[apidoc::param(name = "role_ids", ty = "array", desc = "角色短串数组（整批覆盖）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<AdminUpdateBody>,
) -> Result<(AuditDetail, Json<Value>), ApiError> {
    auth.require("system:admin:edit")?;
    let id = hid::dec(&id)?;
    validate_profile(&body.nickname, &body.email, &body.phone, &body.remark)?;
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    let old = a.clone(); // 变更摘要的旧值（下面就地改 a）

    // 本接口同样能改状态，必须走 set_status 的保护，否则可以绕过它禁用超管/自己。
    // 禁用要在快照比较之外无条件拦：快照可能滞后（库里已被并发启用），只比较快照
    // 会放过「禁用自己 / 禁用超管」。
    let new_status = if body.status == 0 { 0 } else { 1 };
    if new_status == 0 || new_status != a.status {
        if id == auth.admin.id {
            return Err(ApiError::BadRequest("不能修改自己的状态".into()));
        }
        if a.is_super == 1 {
            return Err(ApiError::BadRequest("不能修改超级管理员的状态".into()));
        }
    }

    // 范围与授权闸门（B5）：目标、目标部门、要授的角色三者都不能超出操作者。
    // 放在状态保护之后 —— 「不能改超管/自己状态」是更明确的 400（既有契约，
    // api_flow_test 钉着），范围判定只在它之后兜底。
    datascope::ensure_admin_in_scope(&auth, &state.db, &a).await?;
    datascope::ensure_dept_in_scope(&auth, &state.db, body.dept_id).await?;
    datascope::ensure_roles_added_grantable(&auth, &state.db, id, &body.role_ids).await?;

    a.nickname = body.nickname;
    a.email = sealed(&state.crypto, &body.email)?;
    a.phone = sealed(&state.crypto, &body.phone)?;
    a.sex = body.sex;
    a.dept_id = body.dept_id;
    a.status = new_status;
    a.remark = body.remark;
    a.updated_at = now();

    // 只写本接口负责的列：整行 update 会把读快照里的 password / token_version /
    // totp_secret 写回去，撤销并发的改密或 TOTP 绑定。禁用时版本自增写进同一条
    // SQL —— 用库里的当前值，不用快照（否则并发改密的版本会被压回去）。
    // 不看旧状态：哪怕快照显示已禁用，库里可能刚被并发启用，再写 0 也必须让旧 token 失效。
    let disabling = new_status == 0;
    let sql = if disabling {
        "UPDATE admin SET nickname = ?, email = ?, phone = ?, sex = ?, dept_id = ?, \
         status = ?, remark = ?, updated_at = ?, token_version = token_version + 1 \
         WHERE id = ?"
    } else {
        "UPDATE admin SET nickname = ?, email = ?, phone = ?, sex = ?, dept_id = ?, \
         status = ?, remark = ?, updated_at = ? WHERE id = ?"
    };
    let mut params = vec![
        DbValue::from(a.nickname.clone()),
        DbValue::from(a.email.clone()),
        DbValue::from(a.phone.clone()),
        DbValue::from(a.sex),
        DbValue::from(a.dept_id),
        DbValue::from(a.status),
        DbValue::from(a.remark.clone()),
        DbValue::from(a.updated_at),
    ];
    params.push(DbValue::from(id));
    state.db.execute(sql, &params).await.map_err(ApiError::from)?;

    if disabling {
        // 版本已经作废旧 token，设备行也一起置 0，别让列表里留僵尸设备
        if let Err(e) = state
            .db
            .execute(
                "UPDATE admin_login_device SET status = 0 WHERE admin_id = ? AND status = 1",
                &[DbValue::from(id)],
            )
            .await
        {
            tracing::warn!("禁用管理员后停用设备行失败 (admin_id={id}): {e}");
        }
        crate::auth::forget_admin_devices(id);
    }

    state
        .db
        .set_relations("admin_role", ("admin_id", id), "role_id", &dedup_ids(body.role_ids))
        .await
        .map_err(ApiError::from)?;

    // 变更摘要（项 8）：email / phone 是密文列，只记「改过」（比较明文），值不进摘要
    let changes = Diff::default()
        .field("nickname", old.nickname, a.nickname.clone())
        .masked("email", masked_changed(&state.crypto, &old.email, &body.email))
        .masked("phone", masked_changed(&state.crypto, &old.phone, &body.phone))
        .field("sex", old.sex, a.sex)
        .field("dept_id", hid::enc(old.dept_id.max(0)), hid::enc(a.dept_id.max(0)))
        .field("status", old.status, a.status)
        .field("remark", old.remark, a.remark.clone())
        .done();
    Ok((changes, ok(Value::Null)))
}

#[apidoc::title("删除管理员")]
#[apidoc::desc("连带删管理员-角色关联；不能删自己，也不能删超管")]
#[apidoc::url("/api/v1/admins/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:remove")?;
    let id = hid::dec(&id)?;
    if id == auth.admin.id {
        return Err(ApiError::BadRequest("不能删除自己".into()));
    }
    let a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if a.is_super == 1 {
        return Err(ApiError::BadRequest("不能删除超级管理员".into()));
    }
    datascope::ensure_admin_in_scope(&auth, &state.db, &a).await?;
    state
        .db
        .del_relations("admin_role", "admin_id", id)
        .await
        .map_err(ApiError::from)?;
    // 设备行与备用码是这个人自己的附属数据，不跟着删就是永久孤儿（没有管理员能认领）
    state
        .db
        .del_relations("admin_login_device", "admin_id", id)
        .await
        .map_err(ApiError::from)?;
    state
        .db
        .del_relations("admin_backup_code", "admin_id", id)
        .await
        .map_err(ApiError::from)?;
    a.delete(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("启用 / 禁用管理员")]
#[apidoc::desc("禁用即踢下线（token_version 自增）；不能改自己或超管的状态")]
#[apidoc::url("/api/v1/admins/{id}/status")]
#[apidoc::method("PUT")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::param(name = "status", ty = "int", required, desc = "状态：1 启用 / 0 禁用")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn set_status(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<StatusBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    let id = hid::dec(&id)?;
    if id == auth.admin.id {
        return Err(ApiError::BadRequest("不能修改自己的状态".into()));
    }
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if a.is_super == 1 {
        return Err(ApiError::BadRequest("不能修改超级管理员的状态".into()));
    }
    datascope::ensure_admin_in_scope(&auth, &state.db, &a).await?;
    a.status = if body.status == 0 { 0 } else { 1 };
    a.updated_at = now();
    // 只写 status/updated_at；禁用时 token_version 在 SQL 里用库中当前值自增
    // （整行 update 会拿读快照撤销并发的改密/绑定）。
    let sql = if a.status != 1 {
        "UPDATE admin SET status = ?, updated_at = ?, token_version = token_version + 1 \
         WHERE id = ?"
    } else {
        "UPDATE admin SET status = ?, updated_at = ? WHERE id = ?"
    };
    state
        .db
        .execute(
            sql,
            &[
                DbValue::from(a.status),
                DbValue::from(a.updated_at),
                DbValue::from(id),
            ],
        )
        .await
        .map_err(ApiError::from)?;
    if a.status != 1 {
        // 禁用即踢下线：版本已作废旧 token，设备行也一起置 0
        if let Err(e) = state
            .db
            .execute(
                "UPDATE admin_login_device SET status = 0 WHERE admin_id = ? AND status = 1",
                &[DbValue::from(id)],
            )
            .await
        {
            tracing::warn!("禁用管理员后停用设备行失败 (admin_id={id}): {e}");
        }
        crate::auth::forget_admin_devices(id);
    }
    Ok(ok(Value::Null))
}

#[apidoc::title("重置管理员密码")]
#[apidoc::desc("新密码至少 8 位且含字母/数字/符号中至少两类；旧 token 立即失效（token_version 自增）。不能重置其他超管的密码")]
#[apidoc::url("/api/v1/admins/{id}/password")]
#[apidoc::method("PUT")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::param(name = "password", ty = "string", required, desc = "新密码，至少 8 位且含字母/数字/符号中至少两类")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn reset_password(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<PasswordBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:resetPwd")?;
    let id = hid::dec(&id)?;
    validate_password(&body.password, "密码")?;
    let a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if a.is_super == 1 && id != auth.admin.id {
        return Err(ApiError::BadRequest("不能重置其他超级管理员的密码".into()));
    }
    datascope::ensure_admin_in_scope(&auth, &state.db, &a).await?;
    // 只写 password / token_version / updated_at，并把设备行置 0：旧 token 立即失效
    let password = hash_password_async(body.password).await.map_err(ApiError::internal)?;
    crate::auth::revoke_sessions(&state, id, None, Some(&password)).await?;
    Ok(ok(Value::Null))
}

#[apidoc::title("设置管理员角色")]
#[apidoc::desc("整批覆盖；新增的角色不能超出操作者的数据范围（防自我提权）")]
#[apidoc::url("/api/v1/admins/{id}/roles")]
#[apidoc::method("PUT")]
#[apidoc::tag("管理员")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "管理员对外 id（hashids 短串）")]
#[apidoc::param(name = "role_ids", ty = "array", required, desc = "角色短串数组（整批覆盖，重复项自动去重）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn set_roles(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<RolesBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    let id = hid::dec(&id)?;
    let target = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    datascope::ensure_admin_in_scope(&auth, &state.db, &target).await?;
    // 新增的角色不能比自己宽（自己给自己授 data_scope=1 就是自我提权）；
    // 已挂着的照旧保留，别把「改个昵称」也一起拒了
    datascope::ensure_roles_added_grantable(&auth, &state.db, target.id, &body.role_ids).await?;
    state
        .db
        .set_relations("admin_role", ("admin_id", id), "role_id", &dedup_ids(body.role_ids))
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}
