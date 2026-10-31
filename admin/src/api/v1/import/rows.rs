// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;
use std::collections::HashMap;

/// 单元格（列数不足时当空串：报告里会点名，不用 unwrap 炸掉）。
pub(crate) fn cell(rec: &csv::StringRecord, i: usize) -> &str {
    rec.get(i).unwrap_or("")
}

pub(crate) fn parse_int(s: &str, label: &str) -> Result<i32, ApiError> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(0);
    }
    t.parse().map_err(|_| ApiError::BadRequest(format!("{label}必须是整数，收到 {t:?}")))
}

/// 状态列：空 / 1 / 启用 → 1，0 / 禁用 → 0（与页面上那套中文下拉对齐）。
pub(crate) fn parse_status(s: &str) -> Result<i8, ApiError> {
    match s.trim() {
        "" | "1" | "启用" => Ok(1),
        "0" | "禁用" => Ok(0),
        other => {
            Err(ApiError::BadRequest(format!("状态只认 1/0 或「启用」「禁用」，收到 {other:?}")))
        }
    }
}

/// 按名称定位部门（空 = 根 / 无部门）。重名定位不了就直接报错，不随便挑一个。
pub(crate) fn pick_dept(depts: &[Dept], name: &str, field: &str) -> Result<i64, ApiError> {
    let n = name.trim();
    if n.is_empty() {
        return Ok(0);
    }
    match depts.iter().filter(|d| d.name == n).count() {
        1 => Ok(depts.iter().find(|d| d.name == n).map(|d| d.id).unwrap_or(0)),
        0 => Err(ApiError::BadRequest(format!("{field}「{n}」不存在"))),
        _ => Err(ApiError::BadRequest(format!("{field}「{n}」不唯一，请先把重名的部门改名"))),
    }
}

/// 角色列：名称用 `|` 分隔（与导出的数组列同分隔符）。空 = 不挂角色。
pub(crate) fn pick_roles(roles: &[Role], names: &str) -> Result<Vec<i64>, ApiError> {
    let mut ids = Vec::new();
    for name in names.split('|').map(str::trim).filter(|s| !s.is_empty()) {
        match roles.iter().filter(|r| r.name == name).count() {
            1 => ids.extend(roles.iter().find(|r| r.name == name).map(|r| r.id)),
            0 => return Err(ApiError::BadRequest(format!("角色「{name}」不存在"))),
            _ => return Err(ApiError::BadRequest(format!("角色「{name}」不唯一，请先改名"))),
        }
    }
    Ok(dedup_ids(ids))
}

/// 管理员行。id 在校验期就发号（雪花号有间隔无所谓），写完不用的号作废。
/// **password 此时是明文**（导入内部临时值）：校验阶段完全不 hash，落库前由
/// `write_all` 统一 `spawn_blocking` 批量 hash —— 逐行 argon2 会堵住 async worker。
pub(crate) async fn admin_item(
    state: &AppState,
    auth: &Auth,
    ctx: &mut Ctx,
    rec: &csv::StringRecord,
) -> Result<Item, ApiError> {
    let username = cell(rec, 0).trim().to_string();
    let password = cell(rec, 1);
    let nickname = cell(rec, 2).trim().to_string();
    let email = cell(rec, 3).trim().to_string();
    let phone = cell(rec, 4).trim().to_string();
    let remark = cell(rec, 8).trim().to_string();
    validate_username(&username)?;
    crate::api::admin::validate_password(password, "密码")?;
    validate_profile(&nickname, &email, &phone, &remark)?;
    if ctx.usernames.contains(&username) || !ctx.seen_users.insert(username.clone()) {
        return Err(ApiError::BadRequest("用户名已存在".into()));
    }
    let dept_id = pick_dept(&ctx.depts, cell(rec, 5), "部门")?;
    let role_ids = pick_roles(&ctx.roles, cell(rec, 6))?;
    // 数据范围闸门与同步新建**同一份**函数：导入不能是绕过 B5 的后门
    datascope::ensure_dept_in_scope(auth, &state.db, dept_id).await?;
    datascope::ensure_roles_grantable(auth, &state.db, &role_ids).await?;
    let status = parse_status(cell(rec, 7))?;

    let row = Admin {
        id: state.next_id()?,
        username,
        // 明文临时值，只在导入内存里活到 `write_all` 批量 hash 那一刻
        password: password.to_string(),
        nickname,
        email: sealed(&state.crypto, &email)?,
        phone: sealed(&state.crypto, &phone)?,
        sex: 0,
        avatar: String::new(),
        dept_id,
        status,
        is_super: 0,
        totp_secret: String::new(),
        totp_enabled: 0,
        totp_last_step: None,
        token_version: 0,
        last_login_at: None,
        last_login_ip: String::new(),
        remark,
        created_at: now(),
        updated_at: now(),
    };
    Ok(Item::Admin { row, role_ids })
}

/// 部门行：上级按名称定位，**必须是库里已有的部门**（文件里新建的当不了上级 ——
/// 否则要么一遍扫描里排顺序，要么多趟解析，都不值得）。
pub(crate) fn dept_item(
    state: &AppState,
    ctx: &mut Ctx,
    rec: &csv::StringRecord,
) -> Result<Item, ApiError> {
    let name = cell(rec, 0).trim().to_string();
    let leader = cell(rec, 2).trim().to_string();
    let phone = cell(rec, 3).trim().to_string();
    let sort = parse_int(cell(rec, 4), "排序")?;
    let status = parse_status(cell(rec, 5))?;
    let parent_id = pick_dept(&ctx.depts, cell(rec, 1), "上级部门")?;
    validate_body(&DeptBody {
        parent_id,
        name: name.clone(),
        sort,
        leader: leader.clone(),
        phone: phone.clone(),
        status,
    })?;
    Ok(Item::Dept(Dept {
        id: state.next_id()?,
        parent_id,
        name,
        sort,
        leader,
        phone,
        status,
        created_at: now(),
        updated_at: now(),
    }))
}

/// 字典项行：类型编码必须已存在；`(类型, 值)` 是复合唯一键。
pub(crate) fn dict_item(
    state: &AppState,
    ctx: &mut Ctx,
    rec: &csv::StringRecord,
) -> Result<Item, ApiError> {
    let type_code = cell(rec, 0).trim().to_string();
    let label = cell(rec, 1).trim().to_string();
    let value = cell(rec, 2).trim().to_string();
    let sort = parse_int(cell(rec, 3), "排序")?;
    let status = parse_status(cell(rec, 4))?;
    let remark = cell(rec, 5).trim().to_string();
    check_label(&label)?;
    check_value(&value)?;
    check_len("remark", &remark, 255)?;
    if !ctx.dict_codes.contains(&type_code) {
        return Err(ApiError::BadRequest("字典类型不存在".into()));
    }
    let key = (type_code.clone(), value.clone());
    if ctx.dict_pairs.contains(&key) || !ctx.seen_pairs.insert(key) {
        return Err(ApiError::BadRequest("该类型下字典值已存在".into()));
    }
    Ok(Item::Dict(DictItem {
        id: state.next_id()?,
        type_code,
        label,
        value,
        sort,
        status,
        remark,
        created_at: now(),
        updated_at: now(),
    }))
}

/// 单事务落库（走到这里说明全表都过了校验）。中途任何一步失败 → 回滚，一条不留。
pub(crate) async fn write_all<'a>(
    state: &AppState,
    items: impl Iterator<Item = &'a Item>,
) -> Result<i64, ApiError> {
    let items: Vec<&Item> = items.collect();
    // 明文密码集中到阻塞线程池批量 hash（argon2 很贵）：不占 async worker，
    // 也不会因为事务回滚白算一半行的哈希。校验阶段已经保证这里不会有坏行。
    let plain: Vec<(i64, String)> = items
        .iter()
        .filter_map(|item| match item {
            Item::Admin { row, .. } => Some((row.id, row.password.clone())),
            _ => None,
        })
        .collect();
    let hashed: HashMap<i64, String> = if plain.is_empty() {
        HashMap::new()
    } else {
        tokio::task::spawn_blocking(move || {
            plain.into_iter().map(|(id, p)| (id, hash_password(&p))).collect()
        })
        .await
        .map_err(|e| ApiError::internal(format!("管理员密码批量哈希失败: {e}")))?
    };

    let mut conn = state.db.get().await.map_err(ApiError::from)?;
    conn.begin().await.map_err(ApiError::from)?;
    let mut created = 0i64;
    for item in items {
        let r = match item {
            Item::Admin { row, role_ids } => {
                let password = hashed.get(&row.id).cloned().ok_or_else(|| {
                    ApiError::internal(format!("管理员 {} 的密码哈希缺失", row.id))
                })?;
                let mut row = row.clone();
                row.password = password;
                insert_tx(&mut conn, &row).await.map_err(|e| conflict(e, "用户名已存在"))?;
                for rid in role_ids {
                    conn.execute(
                        "INSERT INTO admin_role (admin_id, role_id) VALUES (?, ?)",
                        &[Value::from(row.id), Value::from(*rid)],
                    )
                    .await
                    .map_err(ApiError::from)?;
                }
                Ok(())
            }
            Item::Dept(d) => insert_tx(&mut conn, d).await,
            Item::Dict(d) => {
                insert_tx(&mut conn, d).await.map_err(|e| conflict(e, "该类型下字典值已存在"))
            }
        };
        if let Err(e) = r {
            // 回滚失败只记日志：原始错误更有用（回滚失败本身会让连接弃用）
            if let Err(re) = conn.rollback().await {
                tracing::error!("导入回滚失败: {re:?}");
            }
            return Err(e);
        }
        created += 1;
    }
    conn.commit().await.map_err(ApiError::from)?;
    Ok(created)
}

/// 唯一键冲突 → 该导入的专属文案：预检之后又被别人占了的窄窗口。
pub(crate) fn conflict(e: ApiError, taken: &str) -> ApiError {
    match e {
        ApiError::BadRequest(_) => ApiError::BadRequest(taken.into()),
        other => other,
    }
}

/// 事务内单行插入。`CheckedConn` 不是 `Db`（见 bee_orm），`Model::insert` 用不了 ——
/// 这里照它的拼法来一份：列名来自模型的 `insert_values`，值一律走绑定参数。
/// 导入的模型都不声明 `auto_now`；哪天有模型声明了，宁可直接报错也不要写出没有时间戳的行。
pub(crate) async fn insert_tx<M: Model>(conn: &mut CheckedConn, row: &M) -> Result<(), ApiError> {
    if !M::auto_now_add_columns().is_empty() || !M::auto_now_columns().is_empty() {
        return Err(ApiError::internal(format!(
            "{} 声明了 auto_now，导入路径不支持（请在 insert_tx 里补时间戳注入）",
            M::table_name()
        )));
    }
    let values = row.insert_values();
    if values.is_empty() {
        return Err(ApiError::internal(format!("{} 没有任何列可写", M::table_name())));
    }
    let columns: Vec<&str> = values.iter().map(|(c, _)| *c).collect();
    let placeholders = vec!["?"; values.len()].join(", ");
    let sql =
        format!("INSERT INTO {} ({}) VALUES ({placeholders})", M::table_name(), columns.join(", "));
    let params: Vec<Value> = values.into_iter().map(|(_, v)| v).collect();
    conn.execute(&sql, &params).await.map_err(ApiError::from)?;
    Ok(())
}
