// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::crypto;
use crate::models::{Admin, Job, Menu, SysConfig};
use crate::util::{hash_password, now};
use bee_orm::migrate;
use bee_orm::pool::mysql::Pool;
use bee_orm::{Model, OrmError, Value};
use snowflake::Shared;

mod defaults;
mod menus;
mod schema;

pub use defaults::{ensure_configs, ensure_jobs};
pub use menus::ensure_menus;
pub use schema::{encrypt_legacy, migrate};

/// 插入前取一个雪花主键。seed 的函数走 `OrmError` 通道（没有 `ApiError`），
/// 发号失败按查询错误上报 —— 进程刚起、表都还没建全，起不来比发出重复 id 好。
fn next_id(sk: &Shared) -> Result<i64, OrmError> {
    sk.id() // snowflake-id-rust 2.x：Shared 的发号方法从 next_id 更名成 id
        .map_err(|e| OrmError::QueryError(format!("雪花发号失败: {e}")))
}

/// 首次启动（admin 表空）时写入超管；内置菜单、任务、配置键由 `ensure_*` 每次启动补齐。
pub async fn seed(
    db: &Pool,
    cfg: &AppConfig,
    sk: &Shared,
    guard: &encryptable::guard::Guard,
) -> Result<(), bee_orm::OrmError> {
    ensure_menus(db, sk).await?;
    ensure_jobs(db, sk).await?;
    ensure_configs(db, sk).await?;

    if Admin::query().count(db).await? > 0 {
        return Ok(());
    }
    // 口令强度只在**真要用它建超管**的这一刻校验：库非空时这个键本就被忽略（上面的
    // return），放在配置加载期拒绝会让存量部署换个口令前整个起不来。口径复用建人
    // 那条（`api::admin::validate_password`，≥8 位 + 至少两类字符）—— 照抄示例配置
    // 装机就是公网后台 admin/admin123，这一步让那种部署死得响亮而不是悄悄上线。
    match crate::api::admin::validate_password(&cfg.initial_admin_password, "initial_admin_password") {
        Ok(()) => {}
        Err(e) => {
            let msg = match e {
                crate::error::ApiError::BadRequest(m) => m,
                other => format!("{other:?}"),
            };
            tracing::error!("[seed] initial_admin_password 太弱：{msg}");
            return Err(OrmError::QueryError(format!(
                "[seed] initial_admin_password 太弱（{msg}）—— admin 表为空时它会成为内置超管的口令，拒绝启动；请换一个至少 8 位、含字母/数字/符号中至少两类的口令"
            )));
        }
    }
    tracing::info!("首次启动，写入种子数据");

    let admin = Admin {
        id: next_id(sk)?,
        username: "admin".into(),
        password: hash_password(&cfg.initial_admin_password),
        nickname: "超级管理员".into(),
        // 空串也照常加密：库里 email/phone 的形态只有一种（密文），读路径不用猜
        email: crypto::write(guard, "").map_err(OrmError::QueryError)?,
        phone: crypto::write(guard, "").map_err(OrmError::QueryError)?,
        sex: 0,
        avatar: String::new(),
        dept_id: 0,
        status: 1,
        is_super: 1,
        totp_secret: String::new(),
        totp_enabled: 0,
        totp_last_step: None,
        token_version: 0,
        last_login_at: None,
        last_login_ip: String::new(),
        remark: "内置超管，请登录后立即修改密码".into(),
        created_at: now(),
        updated_at: now(),
    };
    admin.insert(db).await?;
    // 不打印密码：日志会进 journald/文件，口令可能被长期留存
    tracing::info!("种子完成：已创建超管 admin，初始密码见配置项 initial_admin_password，请登录后立即修改");
    Ok(())
}
