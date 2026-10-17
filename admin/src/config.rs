// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use security_rust::RiskLevel;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("读取配置失败 {path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("配置缺少 [{section}] {key}")]
    Missing { section: String, key: String },
    #[error("配置非法: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub app_name: String,
    pub http_addr: String,
    pub log_level: String,
    pub db_dsn: String,
    pub jwt_secret: String,
    pub jwt_expire_hours: i64,
    pub initial_admin_password: String,
    /// 上传文件根目录（头像等），相对路径相对进程 cwd
    pub upload_dir: String,
    /// 登录失败锁定：窗口内允许的失败次数上限；0 = 关闭锁定
    pub max_fail: i64,
    /// 登录失败锁定时长（分钟）；0 = 关闭锁定
    pub lock_minutes: i64,
    /// 登录图形验证码开关（`[auth] captcha`，poster-rust：点击/旋转/滑块随机切）。
    ///
    /// **默认 true**：登录是唯一没有鉴权的写入口，验证码是加在限流之前的另一道
    /// ——默认关着 = 新建的库裸奔。代价只是每次登录多一次点选/拖动（前端已接线，
    /// 不是「建库即用」的断点）。**关掉（false）时登录接口按老样子工作**：
    /// 不校验凭据、前端也不渲染验证码区。老部署升级时若暂时不重建前端，先设 false。
    pub captcha: bool,
    /// 滑块**拖拽轨迹校验**开关（`[auth] captcha_trajectory` → poster-rust 2.0 的
    /// `captcha.trajectory.enabled`）。**默认 false**：开启后滑块答案必须带轨迹
    /// （插件默认阈值：≥4 个采样点、300–5000ms 总耗时、线性度 ≤0.99），
    /// 只提交裸数值的客户端一律判失败；极快的甩动（<300ms）也会被判机器。
    /// 前端已经**总是**上报轨迹（开关关着时插件直接放行），所以开这个开关不需要重发前端。
    pub captcha_trajectory: bool,
    /// 登录设备保留天数（`[auth] device_keep_days`，默认 30）：**已踢**（软删，status=0）
    /// 的 `admin_login_device` 行留这么久的留痕，之后由 `device_clean` 任务删掉。
    /// **0 = 永久保留**（与 `[log] retain_days` 同一个口径）。在用的行一律不碰。
    pub device_keep_days: i64,
    /// 日志保留天数（login_log / audit_log / job_log）；0 = 永久保留
    pub retain_days: i64,
    /// 定时任务总开关（`[job] enabled`）：false = 不启动调度循环（手动触发仍可用）
    pub job_enabled: bool,
    /// 备份目录（`[backup] dir`）。空 = `{upload_dir}/backups`（见 `crate::backup`）。
    pub backup_dir: String,
    /// 备份保留份数（`[backup] keep`，默认 7）。**必须 >= 1**：0 等于每跑一次就把
    /// 所有备份删光（包括刚生成的那份），几乎必然是误配，拒绝启动。
    pub backup_keep: i64,
    /// 导出文件保留天数（`[export] retain_days`，默认 7）——过期由 export_clean 任务
    /// 删记录 + 文件。**必须 >= 1**：0 等于刚导完就过期，导出中心什么也留不下。
    pub export_retain_days: i64,
    /// 每人的附件总量配额（`[attach] quota_mb`，默认 200）。**0 = 不限**（负数拒启动）：
    /// 口径是 `SUM(size)`，即磁盘占用，不是上传次数（见 `api::attach::check_quota`）。
    pub attach_quota_mb: i64,
    /// 请求安全扫描的拦截阈值（`[security] scan`）：`None` = `off`（**默认**，只报告不拦截）。
    ///
    /// `Some(High)` / `Some(Critical)` = 扫描等级达到该值就 403 + `err = "security.blocked"`
    /// （见 [`crate::security_scan`]）。默认 off 是刻意的：扫描是「有攻击迹象就记一笔」，
    /// 真要挡得先跑一段时间看过误报再定阈值（契约「配置」一节的说明）。
    pub security_scan: Option<RiskLevel>,
    /// 对外 id（hashids）的盐。**换了盐，已发出去的短串全部作废**（前端收藏的链接、
    /// 缓存里的列表都会对不上）；每个部署该是不同的随机串，老配置缺省为空串也能跑。
    pub hashids_salt: String,
    /// 短串最小长度（不够长时补位，短 id 不至于两三字符就可猜）
    pub hashids_min_len: usize,
    /// 雪花节点号与数据中心号（0-31）：多实例部署时各进程必须不同，否则会撞号
    pub snowflake_worker: i64,
    pub snowflake_dc: i64,
    /// `admin.email` / `admin.phone` 的落库加密密钥（`base64:<32 字节>` / 64 位 hex /
    /// 32 字节字面量）。**必填**：缺了直接拒绝启动，不让进程带着「以为加密了其实
    /// 没加密」跑起来。**换了密钥，库里已加密的数据就解不开了**（不可逆，没有救援）。
    pub encrypt_key: String,
    /// 接口文档站（`[apidoc] enabled`），**默认 false**：文档页会把整个后台的接口面
    /// 暴露出来（含请求/响应结构），它是开发/交付工具，不该跟着生产实例一起对外。
    pub apidoc_enabled: bool,
    /// 文档站口令（`[apidoc] password`）。**`enabled = true` 时必填**，缺了拒绝启动
    /// （不允许「以为有口令其实没有」地对外）。
    ///
    /// ⚠️ 插件那套口令的凭据是 `md5(口令)` 放在 query 上 —— 等价于一个**可重放的
    /// 通行证**（抓到一次就能一直用）。所以只在可信网络里开，见 README。
    pub apidoc_password: String,
}

impl AppConfig {
    /// 从 INI 读取；环境变量可覆盖两项（容器/测试注入用）：
    /// - `BEE_ADMIN_DB_DSN` 覆盖 `[db] dsn`
    /// - `BEE_ADMIN_HTTP_ADDR` 覆盖 `[app] http_addr`（容器里必须绑 `0.0.0.0`，
    ///   否则同网络的其他容器连不上：127.0.0.1 是自己的网络命名空间）
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io { path: path.display().to_string(), source: e })?;
        let map = bee_rust::bee_config::ini::IniParser::parse(&content);
        Self::from_ini(&map)
    }

    pub fn from_ini(map: &HashMap<String, HashMap<String, String>>) -> Result<Self, ConfigError> {
        let get = |section: &str, key: &str| -> Result<String, ConfigError> {
            map.get(section)
                .and_then(|s| s.get(key))
                .cloned()
                .ok_or_else(|| ConfigError::Missing {
                    section: section.to_string(),
                    key: key.to_string(),
                })
        };

        let db_dsn = std::env::var("BEE_ADMIN_DB_DSN").unwrap_or(get("db", "dsn")?);
        let jwt_secret = get("jwt", "secret")?;
        if jwt_secret.len() < 32 || jwt_secret == "changeme" {
            return Err(ConfigError::Invalid(
                "[jwt] secret 至少 32 字符且不能是 changeme".into(),
            ));
        }

        // 加密密钥必填（`get` 缺键就是 `[app] encrypt_key` 的缺键错误）。空值 / 占位符
        // 一律拒掉：空密钥会让「以为加密了」的部署把明文当密文写，占位符等于公开密钥。
        let encrypt_key = get("app", "encrypt_key")?;
        if encrypt_key.trim().is_empty() || encrypt_key.contains("change-me") {
            return Err(ConfigError::Invalid(
                "[app] encrypt_key 不能为空或占位符（生产用 `openssl rand -base64 32` 生成，写成 base64:<那串>）".into(),
            ));
        }

        // 可选键：老配置文件没有这些节也照样启动
        let opt = |section: &str, key: &str, default: &str| -> String {
            map.get(section)
                .and_then(|s| s.get(key))
                .cloned()
                .unwrap_or_else(|| default.to_string())
        };
        let num = |section: &str, key: &str, default: i64| -> Result<i64, ConfigError> {
            let raw = opt(section, key, &default.to_string());
            raw.parse()
                .map_err(|_| ConfigError::Invalid(format!("[{section}] {key} 必须是整数")))
        };
        // 布尔开关不接受「非 true/false」的拼写：总开关理解错了（任务默默不跑/照跑）
        // 比启动时报错更难查，直接拒掉。
        let flag = |section: &str, key: &str, default: bool| -> Result<bool, ConfigError> {
            let raw = opt(section, key, if default { "true" } else { "false" });
            match raw.trim().to_ascii_lowercase().as_str() {
                "true" | "1" => Ok(true),
                "false" | "0" => Ok(false),
                other => Err(ConfigError::Invalid(format!(
                    "[{section}] {key} 必须是 true/false，得到 {other}"
                ))),
            }
        };

        // 负数 `as usize` 会绕成天文数字（hashids 拿去补位会当场炸在内存上），先拦掉
        let hashids_min_len = num("app", "hashids_min_len", 8)?;
        if hashids_min_len < 1 {
            return Err(ConfigError::Invalid("[app] hashids_min_len 必须 >= 1".into()));
        }

        // 备份保留份数：0/负数 = 每轮把（含刚生成的）备份全删光，是误配不是意图
        let backup_keep = num("backup", "keep", 7)?;
        if backup_keep < 1 {
            return Err(ConfigError::Invalid(
                "[backup] keep 必须 >= 1（0 等于跑一次就把备份全删了）".into(),
            ));
        }

        // 导出文件保留天数：0 = 刚导完就过期（导出中心白建），是误配不是意图
        let export_retain_days = num("export", "retain_days", 7)?;
        if export_retain_days < 1 {
            return Err(ConfigError::Invalid(
                "[export] retain_days 必须 >= 1（0 等于导出文件立刻过期）".into(),
            ));
        }

        // 附件配额：0 = 不限（合法意图），负数一定是误配 —— 那时 `quota <= 0` 会被
        // 当成「不限」，等于悄悄把闸门全打开，得在启动时说清楚。
        let attach_quota_mb = num("attach", "quota_mb", 200)?;
        if attach_quota_mb < 0 {
            return Err(ConfigError::Invalid(
                "[attach] quota_mb 不能是负数（0 = 不限）".into(),
            ));
        }

        // 文档站开关 + 口令：**enabled = true 却没配口令 = 拒绝启动**。默认 false：
        // 文档站把整个接口面（含请求/响应结构）摊开，是开发/交付工具，不该跟着
        // 生产实例一起对外；真要开，就必须带口令 —— 不允许「以为有口令其实没有」。
        let apidoc_enabled = flag("apidoc", "enabled", false)?;
        let apidoc_password = opt("apidoc", "password", "");
        if apidoc_enabled && apidoc_password.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "[apidoc] password 不能为空（[apidoc] enabled = true 时必填）".into(),
            ));
        }

        // 安全扫描的拦截阈值。取值拼错的后果很实际：「以为在拦其实只报告」（或反过来把
        // 正常用户拦掉）都比启动时报错难查，所以非 off/high/critical 一律拒绝启动
        // —— 与 `[job] enabled` / `[auth] captcha` 一个态度。
        let security_scan = match opt("security", "scan", "off").trim().to_ascii_lowercase().as_str() {
            "off" => None,
            "high" => Some(RiskLevel::High),
            "critical" => Some(RiskLevel::Critical),
            other => {
                return Err(ConfigError::Invalid(format!(
                    "[security] scan 只能是 off/high/critical，得到 {other}"
                )));
            }
        };

        Ok(Self {
            app_name: get("app", "name")?,
            http_addr: std::env::var("BEE_ADMIN_HTTP_ADDR").unwrap_or(get("app", "http_addr")?),
            log_level: get("app", "log_level").unwrap_or_else(|_| "info".into()),
            db_dsn,
            jwt_secret,
            jwt_expire_hours: get("jwt", "expire_hours")?
                .parse()
                .map_err(|_| ConfigError::Invalid("[jwt] expire_hours 必须是整数".into()))?,
            initial_admin_password: get("seed", "initial_admin_password")?,
            upload_dir: opt("app", "upload_dir", "uploads"),
            max_fail: num("auth", "max_fail", 5)?,
            lock_minutes: num("auth", "lock_minutes", 10)?,
            captcha: flag("auth", "captcha", true)?,
            // 轨迹校验默认关：开了老前端（只提交裸数值）会被判失败，不能默认打开
            captcha_trajectory: flag("auth", "captcha_trajectory", false)?,
            // 登录设备留痕：0 = 永久保留（合法意图，与 [log] retain_days 一致），
            // 负数没必要拦（`device_clean` 按 `<= 0` 一视同仁地跳过）
            device_keep_days: num("auth", "device_keep_days", 30)?,
            retain_days: num("log", "retain_days", 90)?,
            job_enabled: flag("job", "enabled", true)?,
            backup_dir: opt("backup", "dir", ""),
            backup_keep,
            export_retain_days,
            attach_quota_mb,
            security_scan,
            hashids_salt: opt("app", "hashids_salt", ""),
            hashids_min_len: hashids_min_len as usize,
            snowflake_worker: num("app", "snowflake_worker", 0)?,
            snowflake_dc: num("app", "snowflake_dc", 0)?,
            encrypt_key,
            apidoc_enabled,
            apidoc_password,
        })
    }
}

#[cfg(test)]
mod tests;
