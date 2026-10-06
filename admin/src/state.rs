// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use bee_orm::Db;
use security_rust::throttle::{MemoryThrottleStore, Throttle};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub cfg: Arc<AppConfig>,
    /// 登录限流闸门（账号维度 + IP 维度）。`None` = 配置关闭了锁定。
    /// 内存实现：进程重启即清零 —— 只影响几分钟的可用性，不影响鉴权正确性。
    /// `login_log` 仍是纯历史记录，限流判定不查它。
    pub throttle: Option<Arc<Throttle<MemoryThrottleStore>>>,
}
