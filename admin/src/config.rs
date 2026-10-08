// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
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
    /// 日志保留天数（login_log / audit_log / job_log）；0 = 永久保留
    pub retain_days: i64,
    /// 定时任务总开关（`[job] enabled`）：false = 不启动调度循环（手动触发仍可用）
    pub job_enabled: bool,
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
            retain_days: num("log", "retain_days", 90)?,
            job_enabled: flag("job", "enabled", true)?,
            hashids_salt: opt("app", "hashids_salt", ""),
            hashids_min_len: hashids_min_len as usize,
            snowflake_worker: num("app", "snowflake_worker", 0)?,
            snowflake_dc: num("app", "snowflake_dc", 0)?,
            encrypt_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[app]
name = bee-rust-admin
http_addr = 127.0.0.1:8080
log_level = info
encrypt_key = base64:YmVlLWFkbWluLXRlc3Qta2V5LTMyLWJ5dGVzLW9rISE=

[db]
dsn = mysql://u:p@127.0.0.1:3306/db

[jwt]
secret = 0123456789012345678901234567890123
expire_hours = 24

[seed]
initial_admin_password = admin123
"#;

    fn parse() -> HashMap<String, HashMap<String, String>> {
        bee_rust::bee_config::ini::IniParser::parse(SAMPLE)
    }

    #[test]
    fn parses_all_fields() {
        let cfg = AppConfig::from_ini(&parse()).unwrap();
        assert_eq!(cfg.app_name, "bee-rust-admin");
        // BEE_ADMIN_HTTP_ADDR 会覆盖 INI 值（容器里绑 0.0.0.0 用），断言两种都认
        let expect_addr =
            std::env::var("BEE_ADMIN_HTTP_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
        assert_eq!(cfg.http_addr, expect_addr);
        // BEE_ADMIN_DB_DSN 会覆盖 INI 值（测试注入真实凭据用），断言两种都认
        let expect = std::env::var("BEE_ADMIN_DB_DSN")
            .unwrap_or_else(|_| "mysql://u:p@127.0.0.1:3306/db".into());
        assert_eq!(cfg.db_dsn, expect);
        assert_eq!(cfg.jwt_expire_hours, 24);
        assert_eq!(cfg.initial_admin_password, "admin123");
    }

    #[test]
    fn optional_keys_default_for_old_confs() {
        // 老 app.conf 没有 [auth] / upload_dir，必须仍能启动
        let cfg = AppConfig::from_ini(&parse()).unwrap();
        assert_eq!(cfg.upload_dir, "uploads");
        assert_eq!(cfg.max_fail, 5);
        assert_eq!(cfg.lock_minutes, 10);
        assert_eq!(cfg.retain_days, 90, "[log] retain_days 默认 90 天");

        let mut map = parse();
        map.insert(
            "auth".into(),
            [("max_fail".to_string(), "3".to_string()), ("lock_minutes".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
        );
        map.get_mut("app").unwrap().insert("upload_dir".into(), "/data/up".into());
        map.insert(
            "log".into(),
            [("retain_days".to_string(), "0".to_string())].into_iter().collect(),
        );
        let cfg = AppConfig::from_ini(&map).unwrap();
        assert_eq!(cfg.upload_dir, "/data/up");
        assert_eq!(cfg.max_fail, 3);
        assert_eq!(cfg.lock_minutes, 1);
        assert_eq!(cfg.retain_days, 0, "0 = 永久保留");
    }

    #[test]
    fn job_switch_defaults_on_and_rejects_garbage() {
        // 老配置文件没有 [job]：默认开着
        assert!(AppConfig::from_ini(&parse()).unwrap().job_enabled);

        let mut map = parse();
        map.insert("job".into(), [("enabled".to_string(), "false".to_string())].into_iter().collect());
        assert!(!AppConfig::from_ini(&map).unwrap().job_enabled, "显式 false 要认");
        map.get_mut("job").unwrap().insert("enabled".into(), "0".into());
        assert!(!AppConfig::from_ini(&map).unwrap().job_enabled, "0 也算 false");
        map.get_mut("job").unwrap().insert("enabled".into(), " TRUE ".into());
        assert!(AppConfig::from_ini(&map).unwrap().job_enabled, "大小写/空白要容错");

        // 拼错的开关不能默默当成 true（任务照跑或照不跑都难查）——直接报错
        map.get_mut("job").unwrap().insert("enabled".into(), "yes".into());
        let err = AppConfig::from_ini(&map).unwrap_err();
        assert!(format!("{err}").contains("[job] enabled"), "应报出具体键: {err}");
    }

    /// 验证码开关：老配置没这个键 = 默认开（登录是唯一无鉴权写入口）；
    /// 显式 false / 0 要认；拼错的开关不能默默当 true（那是「以为有验证码其实没有」）。
    #[test]
    fn captcha_switch_defaults_on_and_rejects_garbage() {
        assert!(AppConfig::from_ini(&parse()).unwrap().captcha, "缺键默认开");

        let mut map = parse();
        map.insert("auth".into(), [("captcha".to_string(), "false".to_string())].into_iter().collect());
        assert!(!AppConfig::from_ini(&map).unwrap().captcha, "显式 false 要认");
        map.get_mut("auth").unwrap().insert("captcha".into(), "0".into());
        assert!(!AppConfig::from_ini(&map).unwrap().captcha, "0 也算 false");
        map.get_mut("auth").unwrap().insert("captcha".into(), " TRUE ".into());
        assert!(AppConfig::from_ini(&map).unwrap().captcha, "大小写/空白要容错");

        map.get_mut("auth").unwrap().insert("captcha".into(), "off".into());
        let err = AppConfig::from_ini(&map).unwrap_err();
        assert!(format!("{err}").contains("[auth] captcha"), "应报出具体键: {err}");
    }

    #[test]
    fn id_keys_default_and_validate() {
        // 老配置没有这几个键：盐为空串、补齐长度 8、节点号 0
        let cfg = AppConfig::from_ini(&parse()).unwrap();
        assert_eq!(cfg.hashids_salt, "");
        assert_eq!(cfg.hashids_min_len, 8);
        assert_eq!((cfg.snowflake_worker, cfg.snowflake_dc), (0, 0));

        let mut map = parse();
        let app = map.get_mut("app").unwrap();
        app.insert("hashids_salt".into(), "s3cret".into());
        app.insert("hashids_min_len".into(), "12".into());
        app.insert("snowflake_worker".into(), "3".into());
        app.insert("snowflake_dc".into(), "2".into());
        let cfg = AppConfig::from_ini(&map).unwrap();
        assert_eq!(cfg.hashids_salt, "s3cret");
        assert_eq!(cfg.hashids_min_len, 12);
        assert_eq!((cfg.snowflake_worker, cfg.snowflake_dc), (3, 2));

        // 0 / 负数长度会让 hashids 补位炸掉；节点号越界由雪花 build() 拒（main 里报启动错）
        map.get_mut("app").unwrap().insert("hashids_min_len".into(), "0".into());
        let err = AppConfig::from_ini(&map).unwrap_err();
        assert!(format!("{err}").contains("hashids_min_len"), "应报出具体键: {err}");
    }

    #[test]
    fn rejects_non_numeric_lock_keys() {
        let mut map = parse();
        map.insert(
            "auth".into(),
            [("max_fail".to_string(), "five".to_string())].into_iter().collect(),
        );
        let err = AppConfig::from_ini(&map).unwrap_err();
        assert!(format!("{err}").contains("[auth] max_fail"), "应报出具体键: {err}");
    }

    #[test]
    fn rejects_short_secret() {
        let mut map = parse();
        map.get_mut("jwt").unwrap().insert("secret".into(), "short".into());
        assert!(AppConfig::from_ini(&map).is_err());
    }

    #[test]
    fn reports_missing_key() {
        let mut map = parse();
        map.remove("db");
        let err = AppConfig::from_ini(&map).unwrap_err();
        assert!(format!("{err}").contains("[db] dsn"));
    }

    /// 加密密钥是必填项：缺了要报出具体键，空值/占位符要拒掉 —— 静默放行等于
    /// 「以为加密了其实没加密」（空密钥）或公开密钥（占位符）。
    #[test]
    fn encrypt_key_is_required_and_not_a_placeholder() {
        assert_eq!(
            AppConfig::from_ini(&parse()).unwrap().encrypt_key,
            "base64:YmVlLWFkbWluLXRlc3Qta2V5LTMyLWJ5dGVzLW9rISE="
        );

        let mut map = parse();
        map.get_mut("app").unwrap().remove("encrypt_key");
        let err = AppConfig::from_ini(&map).unwrap_err();
        assert!(format!("{err}").contains("[app] encrypt_key"), "应报出具体键: {err}");

        for bad in ["", "  ", "base64:change-me-32-random-bytes-here", "change-me"] {
            let mut map = parse();
            map.get_mut("app").unwrap().insert("encrypt_key".into(), bad.into());
            let err = AppConfig::from_ini(&map).unwrap_err();
            assert!(format!("{err}").contains("encrypt_key"), "应拒绝 {bad:?}: {err}");
        }
    }
}
