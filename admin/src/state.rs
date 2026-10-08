// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::error::ApiError;
// 本项目只连 MySQL，直接持具体池类型（`Db` 是 trait，池层才有事务）
use bee_orm::pool::mysql::Pool;
use encryptable::guard::Guard;
use jwt_rust::Jwt;
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
    /// jwt-rust 内核（有状态：密钥 + 配置 + 黑名单存储），启动时按 `[jwt]` 建一次。
    pub jwt: Arc<Jwt>,
    /// admin.email / phone 的加解密守卫（[`crate::crypto`]）。内部是 Arc，克隆廉价。
    pub crypto: Guard,
    /// 登录验证码（poster-rust）。内部是 `Arc<CaptchaManager>`，克隆廉价。
    ///
    /// 存储是插件默认的**进程内** MemoryStorage（TTL 300s）：单实例部署够用，
    /// **多实例要么改插件的 Redis 存储，要么给登录路上的验证码请求做粘性会话**
    /// —— 验证码在 A 实例生成、B 实例校验必然查不到。开关在 `[auth] captcha`
    /// （`false` 时这个守卫还在，只是登录接口不看它）。
    pub captcha: poster::Guard,
    /// 验证码**生成**接口（`/captcha/new`）的限流闸门，恒存在（不像 `throttle` 可配 `None`）：
    /// 插件默认存储不会自动清扫过期 key，不设闸门就是一个公开的内存增长口，
    /// 见 `api::auth::captcha_create_throttle`。
    pub captcha_throttle: Arc<Throttle<MemoryThrottleStore>>,
    /// 请求安全扫描器（security-rust 3.0.0，32 个检测器）。
    ///
    /// 无状态、纯同步，但构造要编译 32 组正则 —— 进程内建一次共享，别按请求建。
    /// 拦截阈值在 `[security] scan`（[`AppConfig::security_scan`]，默认 `None` = 只报告），
    /// 扫描与留痕见 [`crate::security_scan`]。
    pub scanner: Arc<security_rust::Scanner>,
    /// 头像分片上传的运行时（aetherupload-rust 内核 + 进程内秒传表）。
    ///
    /// 构造后不可变、按 `[app] upload_dir` 装配（见 `api::avatar::build_runtime`），
    /// 构造失败**拒绝启动**——别带着半个上传口跑。秒传索引是进程内的：多实例部署下
    /// 各进程各命中各的，只是省流量；重启后第一次上传走完整流程，不影响正确性。
    pub aether: Arc<aetherupload::Runtime>,
}

impl AppState {
    /// 新主键。发号失败（时钟回拨越界 / 序列耗尽）→ 500，不静默用 0。
    pub fn next_id(&self) -> Result<i64, ApiError> {
        self.snowflake
            .next_id()
            .map_err(|e| ApiError::internal(format!("生成 id 失败: {e}")))
    }
}
