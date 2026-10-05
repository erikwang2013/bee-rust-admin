// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::models::{Admin, Menu};
use crate::util::{hash_password, now};
use bee_orm::{Db, Model, SyncdbMode};

/// 建表（syncdb Safe）+ 连接表 DDL，幂等。
pub async fn migrate(db: &Db) -> Result<(), bee_orm::OrmError> {
    db.syncdb(
        &[
            Admin::META,
            crate::models::Dept::META,
            crate::models::Role::META,
            Menu::META,
            crate::models::LoginLog::META,
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

/// 首次启动（admin 表空）时写入：超管 + 菜单权限树。
pub async fn seed(db: &Db, cfg: &AppConfig) -> Result<(), bee_orm::OrmError> {
    if Admin::query().count(db).await? > 0 {
        return Ok(());
    }
    tracing::info!("首次启动，写入种子数据");

    // 种子插入放一个事务：避免中途失败留下「菜单已写、超管未写」的半套数据
    let mut tx = db.begin().await?;

    // ── 菜单：目录 → 菜单 → 按钮 ─────────────────────────────
    let mut system = Menu {
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
    tx.insert(&mut system).await?;

    // (名称, 路径, 组件, 图标, 权限码前缀, 按钮后缀列表)
    let menus: [(&str, &str, &str, &str, &str, &[&str]); 5] = [
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
    ];

    for (i, (name, path, component, icon, prefix, buttons)) in menus.iter().enumerate() {
        let mut m = Menu {
            id: 0,
            parent_id: system.id,
            name: name.to_string(),
            menu_type: "C".into(),
            perm: format!("{prefix}:list"),
            path: path.to_string(),
            component: component.to_string(),
            icon: icon.to_string(),
            sort: (i + 1) as i32,
            visible: 1,
            status: 1,
            created_at: now(),
            updated_at: now(),
        };
        tx.insert(&mut m).await?;
        for (j, action) in buttons.iter().enumerate() {
            if *action == "list" {
                continue; // 菜单自身的 list 权限挂在菜单上
            }
            let label = match *action {
                "add" => "新增",
                "edit" => "编辑",
                "remove" => "删除",
                "resetPwd" => "重置密码",
                other => other,
            };
            let mut b = Menu {
                id: 0,
                parent_id: m.id,
                name: label.to_string(),
                menu_type: "F".into(),
                perm: format!("{prefix}:{action}"),
                path: String::new(),
                component: String::new(),
                icon: String::new(),
                sort: (j + 1) as i32,
                visible: 0,
                status: 1,
                created_at: now(),
                updated_at: now(),
            };
            tx.insert(&mut b).await?;
        }
    }

    // ── 超管 ────────────────────────────────────────────────
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
    tx.insert(&mut admin).await?;
    tx.commit().await?;
    tracing::info!("种子完成：超管 admin / {}", cfg.initial_admin_password);
    Ok(())
}
