// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::models::{Admin, Menu};
use crate::util::{hash_password, now};
use bee_orm::{Db, Model, SyncdbMode};

/// 建表（syncdb Safe）+ 连接表 DDL，幂等。
pub async fn migrate(db: &Db) -> Result<(), bee_orm::OrmError> {
    // dict_item 的复合唯一键 `uk_type_value(type_code, value)` syncdb 表达不了
    // （模型属性只支持单列唯一），所以这张表先裸 DDL 建出来，列与索引再由下面的
    // syncdb 补齐（幂等）。必须建在 syncdb 之前：否则 syncdb 先建出一张没有
    // uk_type_value 的表，IF NOT EXISTS 就再也补不上了。
    db.exec_sql(
        "CREATE TABLE IF NOT EXISTS dict_item (
           id BIGINT UNSIGNED AUTO_INCREMENT NOT NULL,
           type_code VARCHAR(64) NOT NULL DEFAULT '',
           label VARCHAR(64) NOT NULL DEFAULT '',
           value VARCHAR(64) NOT NULL DEFAULT '',
           sort INT NOT NULL DEFAULT 0,
           status TINYINT NOT NULL DEFAULT 0,
           remark VARCHAR(255) NOT NULL DEFAULT '',
           created_at DATETIME NOT NULL,
           updated_at DATETIME NOT NULL,
           PRIMARY KEY (id),
           UNIQUE KEY uk_type_value (type_code, value)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
    )
    .await?;

    db.syncdb(
        &[
            Admin::META,
            crate::models::Dept::META,
            crate::models::Role::META,
            Menu::META,
            crate::models::LoginLog::META,
            crate::models::AuditLog::META,
            crate::models::DictType::META,
            crate::models::DictItem::META,
        ],
        SyncdbMode::Safe,
    )
    .await?;

    // 连接表：syncdb 不支持复合主键，用裸 DDL（幂等）
    for ddl in [
        "CREATE TABLE IF NOT EXISTS admin_role (
           admin_id BIGINT UNSIGNED NOT NULL,
           role_id BIGINT UNSIGNED NOT NULL,
           PRIMARY KEY (admin_id, role_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS role_menu (
           role_id BIGINT UNSIGNED NOT NULL,
           menu_id BIGINT UNSIGNED NOT NULL,
           PRIMARY KEY (role_id, menu_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS role_dept (
           role_id BIGINT UNSIGNED NOT NULL,
           dept_id BIGINT UNSIGNED NOT NULL,
           PRIMARY KEY (role_id, dept_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
    ] {
        db.exec_sql(ddl).await?;
    }
    Ok(())
}

/// 内置菜单定义：(名称, 路径, 组件, 图标, 权限码前缀, 按钮后缀列表)。
const BUILTIN_MENUS: [(&str, &str, &str, &str, &str, &[&str]); 7] = [
    (
        "管理员管理",
        "/system/admin",
        "system/admin/index",
        "UserOutlined",
        "system:admin",
        &["list", "add", "edit", "remove", "resetPwd"],
    ),
    (
        "角色管理",
        "/system/role",
        "system/role/index",
        "TeamOutlined",
        "system:role",
        &["list", "add", "edit", "remove"],
    ),
    (
        "菜单管理",
        "/system/menu",
        "system/menu/index",
        "MenuOutlined",
        "system:menu",
        &["list", "add", "edit", "remove"],
    ),
    (
        "部门管理",
        "/system/dept",
        "system/dept/index",
        "ApartmentOutlined",
        "system:dept",
        &["list", "add", "edit", "remove"],
    ),
    (
        "登录记录",
        "/system/login-log",
        "system/loginlog/index",
        "LoginOutlined",
        "system:loginlog",
        &["list", "remove"],
    ),
    (
        "操作日志",
        "/system/audit-log",
        "system/auditlog/index",
        "FileSearchOutlined",
        "system:auditlog",
        &["list", "remove"],
    ),
    (
        "字典管理",
        "/system/dict",
        "system/dict/index",
        "ProfileOutlined",
        "system:dict",
        &["list", "add", "edit", "remove"],
    ),
];

fn builtin(label: &str) -> String {
    match label {
        "add" => "新增".into(),
        "edit" => "编辑".into(),
        "remove" => "删除".into(),
        "resetPwd" => "重置密码".into(),
        other => other.to_string(),
    }
}

/// 幂等补齐内置菜单树（每次启动都跑）：按权限码查重，缺目录/菜单/按钮就补。
/// 已存在的行一律不改写（管理员改过的名称/排序不被启动流程覆盖）；
/// 反过来说，删掉内置菜单后重启会重新长出来——它们本来就是「内置」。
pub async fn ensure_menus(db: &Db) -> Result<(), bee_orm::OrmError> {
    let all = Menu::query().fetch_all(db).await?;
    let mut seen: std::collections::HashSet<String> =
        all.iter().map(|m| m.perm.clone()).collect();

    // 目录认「类型 M + 路径 /system 或名字系统管理」，管理员改过其中一项也不会重复建
    let is_system_dir = |m: &Menu| m.menu_type == "M" && (m.path == "/system" || m.name == "系统管理");
    let system_id = match all.iter().find(|m| is_system_dir(m)) {
        Some(m) => m.id,
        None => {
            let mut m = Menu {
                id: 0,
                parent_id: 0,
                name: "系统管理".into(),
                menu_type: "M".into(),
                perm: String::new(),
                path: "/system".into(),
                component: String::new(),
                icon: "SettingOutlined".into(),
                sort: 1,
                visible: 1,
                status: 1,
                created_at: now(),
                updated_at: now(),
            };
            db.insert(&mut m).await?;
            m.id
        }
    };

    for (i, (name, path, component, icon, prefix, buttons)) in BUILTIN_MENUS.iter().enumerate() {
        let list_perm = format!("{prefix}:list");
        let parent_id = match all.iter().find(|m| m.perm == list_perm) {
            Some(m) => m.id,
            None => {
                let mut m = Menu {
                    id: 0,
                    parent_id: system_id,
                    name: name.to_string(),
                    menu_type: "C".into(),
                    perm: list_perm.clone(),
                    path: path.to_string(),
                    component: component.to_string(),
                    icon: icon.to_string(),
                    sort: (i + 1) as i32,
                    visible: 1,
                    status: 1,
                    created_at: now(),
                    updated_at: now(),
                };
                db.insert(&mut m).await?;
                seen.insert(list_perm);
                m.id
            }
        };
        for (j, action) in buttons.iter().enumerate() {
            if *action == "list" {
                continue; // 菜单自身的 list 权限挂在菜单上
            }
            let perm = format!("{prefix}:{action}");
            if seen.contains(&perm) {
                continue;
            }
            let mut b = Menu {
                id: 0,
                parent_id,
                name: builtin(action),
                menu_type: "F".into(),
                perm: perm.clone(),
                path: String::new(),
                component: String::new(),
                icon: String::new(),
                sort: (j + 1) as i32,
                visible: 0,
                status: 1,
                created_at: now(),
                updated_at: now(),
            };
            db.insert(&mut b).await?;
            seen.insert(perm);
        }
    }
    Ok(())
}

/// 首次启动（admin 表空）时写入超管；内置菜单由 `ensure_menus` 每次启动补齐。
pub async fn seed(db: &Db, cfg: &AppConfig) -> Result<(), bee_orm::OrmError> {
    ensure_menus(db).await?;

    if Admin::query().count(db).await? > 0 {
        return Ok(());
    }
    tracing::info!("首次启动，写入种子数据");

    let mut admin = Admin {
        id: 0,
        username: "admin".into(),
        password: hash_password(&cfg.initial_admin_password),
        nickname: "超级管理员".into(),
        email: String::new(),
        phone: String::new(),
        sex: 0,
        avatar: String::new(),
        dept_id: 0,
        status: 1,
        is_super: 1,
        token_version: 0,
        last_login_at: None,
        last_login_ip: String::new(),
        remark: "内置超管，请登录后立即修改密码".into(),
        created_at: now(),
        updated_at: now(),
    };
    db.insert(&mut admin).await?;
    // 不打印密码：日志会进 journald/文件，口令可能被长期留存
    tracing::info!("种子完成：已创建超管 admin，初始密码见配置项 initial_admin_password，请登录后立即修改");
    Ok(())
}
