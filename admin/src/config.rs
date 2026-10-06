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

        let mut map = parse();
        map.insert(
            "auth".into(),
            [("max_fail".to_string(), "3".to_string()), ("lock_minutes".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
        );
        map.get_mut("app").unwrap().insert("upload_dir".into(), "/data/up".into());
        let cfg = AppConfig::from_ini(&map).unwrap();
        assert_eq!(cfg.upload_dir, "/data/up");
        assert_eq!(cfg.max_fail, 3);
        assert_eq!(cfg.lock_minutes, 1);
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
}
