// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 内置任务与配置键的默认值。

use super::*;

/// 幂等补齐代码注册的任务（每次启动都跑）：缺哪个 code 补哪个，已存在的行一律不改写
/// （运维改过的间隔/开关不被启动流程覆盖）。库里多出来的旧 code 不删——列表里看得到，
/// 但不可手动触发（409），调度循环也跳过。
pub async fn ensure_jobs(db: &Pool, sk: &Shared) -> Result<(), bee_orm::OrmError> {
    for spec in crate::jobs::JOBS {
        let exists = Job::query()
            .filter_eq("code", spec.code)?
            .one(db)
            .await?
            .is_some();
        if exists {
            continue;
        }
        let j = Job {
            id: next_id(sk)?,
            name: spec.name.to_string(),
            code: spec.code.to_string(),
            cron: spec.interval_secs.to_string(),
            status: 1,
            last_run_at: None,
            last_status: None,
            last_msg: String::new(),
            created_at: now(),
            updated_at: now(),
        };
        j.insert(db).await?;
    }
    Ok(())
}

/// 内置配置键的默认值：(键, 值, 类型, 分组, 备注, 是否公开读)。
/// `site.*` 四个键对应前端原先写死的值（见 `SiteInfo.tsx` / 登录页），公开读给登录页
/// 与页脚用；`notify.*`（项 6）全是内部地址与口令，**必须私有** ——
/// `/configs/public` 是免登录接口，公开等于把 SMTP 服务器、webhook 地址抖给全世界。
/// 版权文字里的年份是**种子时**的年份（字面量，不玩动态替换）：管理员改过一次后
/// 启动流程绝不覆盖，跨年想换自己改。
const DEFAULT_CONFIGS: [(&str, &str, &str, &str, &str, i8); 15] = [
    ("site.copyright", "", "string", "site", "页脚版权文字（原样显示，如 © 2026 erik）", 1),
    (
        "site.repo_url",
        "https://github.com/erikwang2013/bee-rust-admin",
        "string",
        "site",
        "页脚项目地址（整行链接指向它）",
        1,
    ),
    ("site.footer_enabled", "true", "bool", "site", "页脚是否显示（false 时登录页与侧边栏都不渲染）", 1),
    ("site.login_subtitle", "", "string", "site", "登录页副标题（空 = 不显示）", 1),
    // ── 通知渠道（项 6）：键名与 `src/notify.rs` 的常量同源，改名只改常量 ──
    (crate::notify::KEY_WEBHOOK_ENABLED, "false", "bool", "notify", "Webhook 总开关（事件：公告发布 / 账号锁定）", 0),
    (crate::notify::KEY_WEBHOOK_URL, "", "string", "notify", "Webhook 地址（http/https，超时 5s；失败落库由 notify_retry 重投）", 0),
    (
        crate::notify::KEY_WEBHOOK_SECRET,
        "",
        "string",
        "notify",
        // 这句备注是给「直接翻库」的人看的：值已经被 notify 接口加密存了
        "Webhook HMAC 签名密钥（AES-256-GCM 密文；由「通知设置」页维护，通用配置页不列此键）",
        0,
    ),
    (crate::notify::KEY_SMTP_ENABLED, "false", "bool", "notify", "SMTP 总开关（测试邮件与公告邮件推送都走它）", 0),
    (crate::notify::KEY_SMTP_HOST, "", "string", "notify", "SMTP 服务器地址", 0),
    (crate::notify::KEY_SMTP_PORT, "587", "string", "notify", "SMTP 端口（starttls 587 / ssl 465）", 0),
    (crate::notify::KEY_SMTP_USER, "", "string", "notify", "SMTP 认证用户名（空 = 不认证）", 0),
    (
        crate::notify::KEY_SMTP_PASSWORD,
        "",
        "string",
        "notify",
        // 这句备注是给「直接翻库」的人看的：值已经被 notify 接口加密存了
        "SMTP 认证口令（AES-256-GCM 密文；由「通知设置」页维护，通用配置页不列此键）",
        0,
    ),
    (crate::notify::KEY_SMTP_FROM, "", "string", "notify", "发件人地址（空 = 用认证用户名）", 0),
    (crate::notify::KEY_SMTP_ENCRYPTION, "starttls", "string", "notify", "加密方式：starttls / ssl / none", 0),
    (
        crate::notify::KEY_EMAIL_NOTICE_ENABLED,
        "false",
        "bool",
        "notify",
        "公告发布时给全体启用且填了邮箱的账号发邮件（发布者本人除外），默认关",
        0,
    ),
];

/// 幂等补齐内置配置键（每次启动都跑）：缺哪个补哪个，已存在的行一律不改写
/// （管理员改过的值不被启动流程覆盖）；库里多出来的键不删——管理页可见可删。
pub async fn ensure_configs(db: &Pool, sk: &Shared) -> Result<(), bee_orm::OrmError> {
    for (key, value, ty, group, remark, is_public) in DEFAULT_CONFIGS {
        let exists = SysConfig::query()
            .filter_eq("config_key", key)?
            .one(db)
            .await?
            .is_some();
        if exists {
            continue;
        }
        // 版权默认值在种子这一刻取当前年，拼成字面量（见 `DEFAULT_CONFIGS` 注释）
        let value = if key == "site.copyright" {
            format!("© {} erik", chrono::Local::now().format("%Y"))
        } else {
            value.to_string()
        };
        let c = SysConfig {
            id: next_id(sk)?,
            key: key.to_string(),
            value,
            value_type: ty.to_string(),
            group: group.to_string(),
            is_public,
            remark: remark.to_string(),
            created_at: now(),
            updated_at: now(),
        };
        c.insert(db).await?;
    }
    Ok(())
}
