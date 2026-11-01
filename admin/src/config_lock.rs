// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 配置写锁：角色/部门/菜单/管理员的删除与关联分配不能同时发生。
//!
//! 这些表的删除接口是「先查引用，再删父行」，检查与写入之间若插入一条关联，
//! 就会留下指向已删对象的悬挂行。`log_retention` 的兜底清理能收掉历史脏行，
//! 但这把进程内的锁让**新的**配置写入彼此串行，不再制造新的窗口。
//!
//! shortcut: 只在单实例内生效（本项目的 Compose 是单后端）；多实例部署要换
//! DB 行锁 / 外键。锁只覆盖低频配置写，不覆盖读接口与业务数据。
use tokio::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::const_new(());

pub async fn lock() -> MutexGuard<'static, ()> {
    LOCK.lock().await
}
