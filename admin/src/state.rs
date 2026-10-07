// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::error::ApiError;
// 本项目只连 MySQL，直接持具体池类型（`Db` 是 trait，池层才有事务）
use bee_orm::pool::mysql::Pool;
use security_rust::throttle::{MemoryThrottleStore, Throttle};
use snowflake::Shared;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: Pool,
    pub cfg: Arc<AppConfig>,
    /// 登录限流闸门（账号维度 + IP 维度）。`None` = 配置关闭了锁定。
    /// 内存实现：进程重启即清零 —— 只影响几分钟的可用性，不影响鉴权正确性。
    /// `login_log` 仍是纯历史记录，限流判定不查它。
    pub throttle: Option<Arc<Throttle<MemoryThrottleStore>>>,
    /// 雪花发号器：主键由代码给出（模型上没有 `#[bee(auto)]`），插入前取号。
    /// `Arc<Mutex<_>>` 壳子里只有一份生成器状态，克隆廉价。
    pub snowflake: Shared,
}

impl AppState {
    /// 新主键。发号失败（时钟回拨越界 / 序列耗尽）→ 500，不静默用 0。
    pub fn next_id(&self) -> Result<i64, ApiError> {
        self.snowflake
            .next_id()
            .map_err(|e| ApiError::internal(format!("生成 id 失败: {e}")))
    }
}
