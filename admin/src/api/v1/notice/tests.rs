// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;

fn at(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
}

#[test]
fn published_at_only_set_on_first_publish() {
    let was = at("2026-01-01 08:00:00");
    // 首次发布：写
    assert!(publish_at(1, None).is_some());
    // 重复发布：保留旧值（不是 now()）
    assert_eq!(publish_at(1, Some(was)), Some(was));
    // 撤回：不清空（留痕：曾发布过）
    assert_eq!(publish_at(0, Some(was)), Some(was));
    // 草稿从未发布过：保持空
    assert_eq!(publish_at(0, None), None);
}

#[test]
fn title_and_content_rejected_when_blank_or_oversize() {
    assert!(check_title("  ").is_err());
    assert!(check_title("维护通知").is_ok());
    assert!(check_title(&"x".repeat(129)).is_err());
    assert!(check_title(&"x".repeat(128)).is_ok());
    assert!(check_content(" ").is_err());
    assert!(check_content("今晚停机").is_ok());
    assert!(check_content(&"x".repeat(CONTENT_MAX + 1)).is_err());
}

/// 三个写入口（create / update / set_status）共用这一条：0/1 之外的都 400。
#[test]
fn status_must_be_draft_or_published() {
    assert!(check_status(0).is_ok());
    assert!(check_status(1).is_ok());
    for bad in [-1, 2, 127] {
        match check_status(bad) {
            Err(ApiError::BadRequest(msg)) => assert!(msg.contains("0 或 1"), "{msg}"),
            other => panic!("状态 {bad} 应返回 400，实际: {other:?}"),
        }
    }
}

/// 字符数没超、字节数超了也必须拒：4 字节 emoji × 16384 = 65536 字节 > TEXT 上限。
#[test]
fn content_byte_limit_rejects_multibyte_overflow() {
    let emoji = "😀".repeat(CONTENT_MAX_BYTES / 4 + 1);
    assert!(emoji.chars().count() < CONTENT_MAX, "字符数本身没超，超的是字节数");
    match check_content(&emoji) {
        Err(ApiError::BadRequest(msg)) => assert!(msg.contains("字节"), "{msg}"),
        other => panic!("应返回 400 字节超限，实际: {other:?}"),
    }
    // 中文 3 字节：正常长度仍能过
    assert!(check_content("今晚停机维护，预计两小时").is_ok());
}
