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
