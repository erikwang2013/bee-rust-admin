// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

use super::*;

/// 只为 `build_runtime` 取 `upload_dir`；其余字段给能过配置校验的占位值。
fn test_config() -> AppConfig {
    const SAMPLE: &str = r#"
[app]
name = bee-rust-admin-test
http_addr = 127.0.0.1:0
upload_dir = target/test-uploads
encrypt_key = base64:YmVlLWFkbWluLXRlc3Qta2V5LTMyLWJ5dGVzLW9rISE=

[db]
dsn = mysql://u:p@127.0.0.1:3306/db_test

[jwt]
secret = 0123456789012345678901234567890123
expire_hours = 1

[seed]
initial_admin_password = admin123
"#;
    let map = bee_rust::bee_config::ini::IniParser::parse(SAMPLE);
    AppConfig::from_ini(&map).expect("样例配置应当合法")
}

#[test]
fn runtime_pins_the_upload_root_at_upload_dir() {
    let cfg = test_config();
    let runtime = build_runtime(&cfg).expect("构造应当成功");

    // `root_dir` = upload_dir + `base_path` = ""：上传根就是 [app] upload_dir
    assert_eq!(runtime.upload_root(), Path::new("target/test-uploads"));
    // 插件不会自己建这两个目录（缺了第一个上传就失败），构造时必须建好
    assert!(Path::new("target/test-uploads/avatar").is_dir());
    assert!(Path::new("target/test-uploads/_header").is_dir());

    // 分组配置：只此一个分组（默认的 `file` 被换掉），白名单与硬闸都对
    let snapshot = runtime.config().resolve_group(AVATAR_GROUP).expect("avatar 分组存在");
    assert_eq!(snapshot.group_dir, "avatar");
    assert_eq!(snapshot.resource_maxsize, AVATAR_MAX_BYTES);
    assert_eq!(snapshot.resource_extensions, vec!["png".to_string(), "jpg".to_string()]);
    assert!(snapshot.instant_completion, "秒传要开着（同一张图重传命中）");
    assert_eq!(
        snapshot.chunk_size, AVATAR_CHUNK_BYTES,
        "分片必须小于头像上限，否则分片路径永远走不到"
    );
    assert!(
        snapshot.chunk_size < AVATAR_MAX_BYTES,
        "契约：chunk_size < 头像上限"
    );
    assert!(
        runtime.config().resolve_group("file").is_err(),
        "默认的 file 分组不该留下（那是往别的目录写大文件的路径）"
    );
}

#[test]
fn public_avatar_hides_the_saved_path() {
    // hid 的全局单例在单测里没人初始化（main 才调）：本用例要编短串，自己来一次。
    // OnceLock 只能初始化一次，别的用例先跑就复用它的盐（与 hid.rs 的 setup 同一套路）。
    let _ = crate::hid::init("hid-unit-test-salt", 8);
    // 存的是 saved_path（内部地址），对外的形状仍是 /api/v1/avatar/{短串}
    assert_eq!(
        public_avatar("avatar_202610_d41d8cd98f00b204e9800998ecf8427e.png", 7),
        avatar_url(7)
    );
    // 老 URL 形态（4b 之前的存量值）同样换成当前形状：两者本来就是同一个地址
    assert_eq!(public_avatar("/api/v1/avatar/abc", 7), avatar_url(7));
    // 没有头像：仍是空串（给个必然 404 的地址不如让前端显示占位）
    assert_eq!(public_avatar("", 7), "");
    assert_eq!(public_avatar("   ", 7), "");
}

#[test]
fn image_content_type_follows_the_extension() {
    assert_eq!(image_content_type(Path::new("/x/a.png")), Some("image/png"));
    assert_eq!(image_content_type(Path::new("/x/a.JPG")), Some("image/jpeg"));
    assert_eq!(image_content_type(Path::new("/x/a.jpeg")), Some("image/jpeg"));
    assert_eq!(image_content_type(Path::new("/x/a.gif")), None);
    assert_eq!(image_content_type(Path::new("/x/a")), None);
}
