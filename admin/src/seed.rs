// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::models::{Admin, Job, Menu};
use crate::util::{hash_password, now};
use bee_orm::migrate;
use bee_orm::pool::mysql::Pool;
use bee_orm::{Model, OrmError, Value};
use snowflake::Shared;

/// 插入前取一个雪花主键。seed 的函数走 `OrmError` 通道（没有 `ApiError`），
/// 发号失败按查询错误上报 —— 进程刚起、表都还没建全，起不来比发出重复 id 好。
fn next_id(sk: &Shared) -> Result<i64, OrmError> {
    sk.next_id().map_err(|e| OrmError::QueryError(format!("雪花发号失败: {e}")))
}

/// 建表 + 补列（只在启动时跑，幂等）。
///
/// 从本地自写的 `syncdb` 换到上游的 `migrate` 之后，**约束不再由模型属性声明**：
/// 上游的字段属性只有 `pk / auto / ignore / auto_now_add / auto_now / soft_delete /
/// sql_type / fk` —— 没有 unique、没有 index、也没有列长度。所以：
///   - 列长度改用 `#[bee(sql_type = "VARCHAR(n)")]`（`SqlType::Raw` 原样渲染）
///   - **唯一键与索引改由这里建**（下方 `CONSTRAINTS`），与既有的连接表、复合唯一键同一套做法
/// 命名沿用迁移前 syncdb 的约定（`uk_{表}_{列}` / `idx_{表}_{列}`）——名字变了会在已有库上
/// 重复建一份同名不同名的索引。
pub async fn migrate(db: &Pool) -> Result<(), OrmError> {
    // 1) 复合主键/复合唯一键的表：上游 migrate 只建单列主键，表达不了，走裸 DDL。
    //    必须建在 sync 之前：否则 sync 先建出一张没有复合约束的表，IF NOT EXISTS 就再补不上。
    //    `id` 一律不带 AUTO_INCREMENT：主键由代码发号（雪花），列定义要与 migrate 生成的
    //    其它表一致 —— 留着自增会让人以为这里还能靠数据库发号（旧的库上该列仍是自增的，
    //    但应用侧总是显式给值，无害）。
    for ddl in [
        "CREATE TABLE IF NOT EXISTS dict_item (
           id BIGINT NOT NULL,
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
        "CREATE TABLE IF NOT EXISTS job_log (
           id BIGINT NOT NULL,
           job_code VARCHAR(64) NOT NULL DEFAULT '',
           started_at DATETIME NOT NULL,
           duration_ms BIGINT NOT NULL DEFAULT 0,
           status TINYINT NOT NULL DEFAULT 0,
           msg VARCHAR(255) NOT NULL DEFAULT '',
           PRIMARY KEY (id),
           KEY idx_code_time (job_code, started_at)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS admin_role (
           admin_id BIGINT NOT NULL,
           role_id BIGINT NOT NULL,
           PRIMARY KEY (admin_id, role_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS role_menu (
           role_id BIGINT NOT NULL,
           menu_id BIGINT NOT NULL,
           PRIMARY KEY (role_id, menu_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS role_dept (
           role_id BIGINT NOT NULL,
           dept_id BIGINT NOT NULL,
           PRIMARY KEY (role_id, dept_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        // 公告已读：同一公告同一人只有一行（重复标记靠 INSERT IGNORE 幂等）
        "CREATE TABLE IF NOT EXISTS notice_read (
           notice_id BIGINT NOT NULL,
           admin_id BIGINT NOT NULL,
           read_at DATETIME NOT NULL,
           PRIMARY KEY (notice_id, admin_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
    ] {
        db.execute(ddl, &[]).await?;
    }

    // 2) 各模型：建表 + 补缺列，绝不删列、不改类型（与迁移前的 Safe 模式一致）
    migrate::sync::<Admin, _>(db).await?;
    migrate::sync::<crate::models::Dept, _>(db).await?;
    migrate::sync::<crate::models::Role, _>(db).await?;
    migrate::sync::<Menu, _>(db).await?;
    migrate::sync::<crate::models::LoginLog, _>(db).await?;
    migrate::sync::<crate::models::AuditLog, _>(db).await?;
    migrate::sync::<crate::models::DictType, _>(db).await?;
    migrate::sync::<crate::models::DictItem, _>(db).await?;
    migrate::sync::<Job, _>(db).await?;
    migrate::sync::<crate::models::JobLog, _>(db).await?;
    migrate::sync::<crate::models::Notice, _>(db).await?;

    // 3) 单列唯一键与索引：原先写在模型属性上（`#[bee(unique)]` / `#[bee(index)]`），
    //    上游属性里没有这两个概念，移到这儿。
    for (table, index, ddl) in CONSTRAINTS {
        ensure_index(db, table, index, ddl).await?;
    }
    Ok(())
}

/// 从模型属性搬过来的约束。名字必须与迁移前 syncdb 生成的一致。
const CONSTRAINTS: &[(&str, &str, &str)] = &[
    ("admin", "uk_admin_username", "CREATE UNIQUE INDEX uk_admin_username ON admin (username)"),
    ("admin", "idx_admin_dept_id", "CREATE INDEX idx_admin_dept_id ON admin (dept_id)"),
    ("admin", "idx_admin_status", "CREATE INDEX idx_admin_status ON admin (status)"),
    ("role", "uk_role_code", "CREATE UNIQUE INDEX uk_role_code ON role (code)"),
    ("dict_type", "uk_dict_type_code", "CREATE UNIQUE INDEX uk_dict_type_code ON dict_type (code)"),
    ("dict_item", "idx_dict_item_type_code", "CREATE INDEX idx_dict_item_type_code ON dict_item (type_code)"),
    ("job", "uk_job_code", "CREATE UNIQUE INDEX uk_job_code ON job (code)"),
    ("job_log", "idx_job_log_job_code", "CREATE INDEX idx_job_log_job_code ON job_log (job_code)"),
];

/// 幂等建索引：MySQL 没有 `CREATE INDEX IF NOT EXISTS`，先查 information_schema。
async fn ensure_index(db: &Pool, table: &str, index: &str, ddl: &str) -> Result<(), OrmError> {
    let rows = db
        .query(
            "SELECT COUNT(*) AS n FROM information_schema.statistics \
             WHERE table_schema = DATABASE() AND table_name = ? AND index_name = ?",
            &[Value::from(table), Value::from(index)],
        )
        .await?;
    let n = rows
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if n == 0 {
        db.execute(ddl, &[]).await?;
    }
    Ok(())
}

/// 内置菜单定义：(名称, 路径, 组件, 图标, 权限码前缀, 按钮后缀列表)。
const BUILTIN_MENUS: [(&str, &str, &str, &str, &str, &[&str]); 9] = [
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
    // 任务只读 + 改间隔/开关 + 手动触发：没有新增/删除（任务是代码注册的）
    (
        "定时任务",
        "/system/job",
        "system/job/index",
        "ClockCircleOutlined",
        "system:job",
        &["list", "edit"],
    ),
    (
        "通知公告",
        "/system/notice",
        "system/notice/index",
        "BellOutlined",
        "system:notice",
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
pub async fn ensure_menus(db: &Pool, sk: &Shared) -> Result<(), bee_orm::OrmError> {
    let all = Menu::query().all(db).await?;
    let mut seen: std::collections::HashSet<String> =
        all.iter().map(|m| m.perm.clone()).collect();

    // 目录认「类型 M + 路径 /system 或名字系统管理」，管理员改过其中一项也不会重复建
    let is_system_dir = |m: &Menu| m.menu_type == "M" && (m.path == "/system" || m.name == "系统管理");
    let system_id = match all.iter().find(|m| is_system_dir(m)) {
        Some(m) => m.id,
        None => {
            let m = Menu {
                id: next_id(sk)?,
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
            // id 由上面发号给出，insert 就够了（create 的读回是给自增主键用的）
            m.insert(db).await?;
            m.id
        }
    };

    for (i, (name, path, component, icon, prefix, buttons)) in BUILTIN_MENUS.iter().enumerate() {
        let list_perm = format!("{prefix}:list");
        let parent_id = match all.iter().find(|m| m.perm == list_perm) {
            Some(m) => m.id,
            None => {
                let m = Menu {
                    id: next_id(sk)?,
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
                m.insert(db).await?;
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
            let b = Menu {
                id: next_id(sk)?,
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
            b.insert(db).await?;
            seen.insert(perm);
        }
    }
    Ok(())
}

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

/// 首次启动（admin 表空）时写入超管；内置菜单与任务由 `ensure_*` 每次启动补齐。
pub async fn seed(db: &Pool, cfg: &AppConfig, sk: &Shared) -> Result<(), bee_orm::OrmError> {
    ensure_menus(db, sk).await?;
    ensure_jobs(db, sk).await?;

    if Admin::query().count(db).await? > 0 {
        return Ok(());
    }
    tracing::info!("首次启动，写入种子数据");

    let admin = Admin {
        id: next_id(sk)?,
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
    admin.insert(db).await?;
    // 不打印密码：日志会进 journald/文件，口令可能被长期留存
    tracing::info!("种子完成：已创建超管 admin，初始密码见配置项 initial_admin_password，请登录后立即修改");
    Ok(())
}
