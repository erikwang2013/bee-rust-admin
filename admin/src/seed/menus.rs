// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 内置菜单树。

use super::*;

/// 内置菜单定义：(名称, 路径, 组件, 图标, 权限码前缀, 按钮后缀列表)。
const BUILTIN_MENUS: [(&str, &str, &str, &str, &str, &[&str]); 13] = [
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
    // 写动作（改值/增键/删键）都归 `system:config:edit` 一个码：页面是分组表单，
    // 不是「行级增删改」那种按钮矩阵，拆细反而没人配得清
    (
        "系统配置",
        "/system/config",
        "system/config/index",
        "ToolOutlined",
        "system:config",
        &["list", "edit"],
    ),
    // 通知设置（项 6）：Webhook 与 SMTP 同页配。「存配置」与「发测试邮件」都归 edit
    // —— 页面上没有行级增删改，拆细没人配得清（与系统配置同一取舍）
    (
        "通知设置",
        "/system/notify",
        "system/notify/index",
        "NotificationOutlined",
        "system:notify",
        &["list", "edit"],
    ),
    // 导出中心（项 4）：发起/下载按各类型自己的列表码，这里只有「进得来」与「删得掉」
    (
        "导出中心",
        "/system/export",
        "system/export/index",
        "DownloadOutlined",
        "system:export",
        &["list", "remove"],
    ),
    // 附件中心（项 5）：edit 管「关联业务对象」（含解绑），remove 管删除；
    // 下载本身不发独立的码 —— 能进列表 + 数据权限就够，多一个码只会多一处配错
    (
        "附件中心",
        "/system/attach",
        "system/attach/index",
        "FolderOpenOutlined",
        "system:attach",
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
