// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

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

/// 备份保留份数：缺键默认 7；0/负数拒绝启动（跑一次就把备份全删是误配）；
/// `[backup] dir` 缺省空串 = 落到 `{upload_dir}/backups`。
#[test]
fn backup_keep_defaults_and_rejects_nonpositive() {
    let cfg = AppConfig::from_ini(&parse()).unwrap();
    assert_eq!(cfg.backup_keep, 7, "缺 [backup] = 保留 7 份");
    assert_eq!(cfg.backup_dir, "", "缺省 = 空串（落到 upload_dir/backups）");

    let mut map = parse();
    map.insert(
        "backup".into(),
        [("keep".to_string(), "3".to_string()), ("dir".to_string(), "/data/bak".to_string())]
            .into_iter()
            .collect(),
    );
    let cfg = AppConfig::from_ini(&map).unwrap();
    assert_eq!(cfg.backup_keep, 3);
    assert_eq!(cfg.backup_dir, "/data/bak");

    for bad in ["0", "-1"] {
        let mut map = parse();
        map.insert(
            "backup".into(),
            [("keep".to_string(), bad.to_string())].into_iter().collect(),
        );
        let err = AppConfig::from_ini(&map).unwrap_err();
        assert!(format!("{err}").contains("[backup] keep"), "应点名 [backup] keep: {err}");
    }
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

/// 轨迹校验开关：与 `[auth] captcha` 共用 `flag()`，但**默认关**
/// （插件侧默认也是 false；开了老前端/脚本的裸数值答案会被判 400，不能默认打开）。
#[test]
fn captcha_trajectory_defaults_off_and_accepts_flags() {
    assert!(!AppConfig::from_ini(&parse()).unwrap().captcha_trajectory, "缺键默认关");

    let mut map = parse();
    map.insert(
        "auth".into(),
        [("captcha_trajectory".to_string(), "true".to_string())].into_iter().collect(),
    );
    assert!(AppConfig::from_ini(&map).unwrap().captcha_trajectory, "显式 true 要认");
    map.get_mut("auth").unwrap().insert("captcha_trajectory".into(), "1".into());
    assert!(AppConfig::from_ini(&map).unwrap().captcha_trajectory, "1 也算 true");
    map.get_mut("auth").unwrap().insert("captcha_trajectory".into(), " FALSE ".into());
    assert!(!AppConfig::from_ini(&map).unwrap().captcha_trajectory, "大小写/空白要容错");
    map.get_mut("auth").unwrap().insert("captcha_trajectory".into(), "0".into());
    assert!(!AppConfig::from_ini(&map).unwrap().captcha_trajectory, "0 也算 false");

    // 拼错的开关拒绝启动：默默当 true 会把老前端全挡在门外，当 false 又「以为在防机器」
    map.get_mut("auth").unwrap().insert("captcha_trajectory".into(), "on".into());
    let err = AppConfig::from_ini(&map).unwrap_err();
    assert!(format!("{err}").contains("[auth] captcha_trajectory"), "应报出具体键: {err}");
}

/// 安全扫描阈值：老配置没有 [security] = 默认 off（只报告不拦截）；
/// high / critical 认大小写与空白；**拼错的取值拒绝启动** —— 「以为在拦其实只报告」
/// 或「以为只报告其实在拦」都是事故，不如起不来。
#[test]
fn security_scan_defaults_off_and_rejects_garbage() {
    assert_eq!(AppConfig::from_ini(&parse()).unwrap().security_scan, None, "缺键 = off");

    let mut map = parse();
    map.insert("security".into(), [("scan".to_string(), "high".to_string())].into_iter().collect());
    assert_eq!(AppConfig::from_ini(&map).unwrap().security_scan, Some(RiskLevel::High));
    map.get_mut("security").unwrap().insert("scan".into(), " CRITICAL ".into());
    assert_eq!(AppConfig::from_ini(&map).unwrap().security_scan, Some(RiskLevel::Critical));
    map.get_mut("security").unwrap().insert("scan".into(), "off".into());
    assert_eq!(AppConfig::from_ini(&map).unwrap().security_scan, None);

    map.get_mut("security").unwrap().insert("scan".into(), "on".into());
    let err = AppConfig::from_ini(&map).unwrap_err();
    assert!(format!("{err}").contains("[security] scan"), "应报出具体键: {err}");
}

/// 文档站开关：老配置没有 [apidoc] = 默认关（不能凭空开出一个暴露接口面的站点）；
/// **开了却没口令 = 拒绝启动**（「以为有口令其实没有」）；拼错的开关同样拒绝。
#[test]
fn apidoc_defaults_off_and_requires_password_when_on() {
    let cfg = AppConfig::from_ini(&parse()).unwrap();
    assert!(!cfg.apidoc_enabled, "缺 [apidoc] 节 = 默认关");
    assert_eq!(cfg.apidoc_password, "");

    // enabled = true + 口令：认
    let mut map = parse();
    map.insert(
        "apidoc".into(),
        [("enabled".to_string(), "true".to_string()), ("password".to_string(), "s3cret".to_string())]
            .into_iter()
            .collect(),
    );
    let cfg = AppConfig::from_ini(&map).unwrap();
    assert!(cfg.apidoc_enabled);
    assert_eq!(cfg.apidoc_password, "s3cret");

    // enabled = true 但口令缺键 / 空 / 全空白：一律拒绝启动（点名 apidoc password）
    for password in [None, Some(""), Some("   ")] {
        let mut map = parse();
        let mut sec: HashMap<String, String> =
            [("enabled".to_string(), "true".to_string())].into_iter().collect();
        if let Some(p) = password {
            sec.insert("password".into(), p.into());
        }
        map.insert("apidoc".into(), sec);
        let err = AppConfig::from_ini(&map).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("apidoc") && msg.contains("password"),
            "应点名 [apidoc] password: {msg}"
        );
    }

    // 关着的时候口令空着无所谓（默认部署就是这样）
    let mut map = parse();
    map.insert("apidoc".into(), [("enabled".to_string(), "false".to_string())].into_iter().collect());
    assert!(!AppConfig::from_ini(&map).unwrap().apidoc_enabled);

    // 拼错的开关不能默默当 false（文档站照旧关着没人查）——直接报错
    map.get_mut("apidoc").unwrap().insert("enabled".into(), "yes".into());
    let err = AppConfig::from_ini(&map).unwrap_err();
    assert!(format!("{err}").contains("[apidoc] enabled"), "应报出具体键: {err}");
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
