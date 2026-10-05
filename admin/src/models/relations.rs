// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use serde::{Deserialize, Serialize};

// 连接表模型：只用于计数/查询，建表走裸 DDL（syncdb 不支持复合主键）

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "admin_role")]
pub struct AdminRole {
    pub admin_id: u64,
    pub role_id: u64,
}

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "role_menu")]
pub struct RoleMenu {
    pub role_id: u64,
    pub menu_id: u64,
}

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "role_dept")]
pub struct RoleDept {
    pub role_id: u64,
    pub dept_id: u64,
}
