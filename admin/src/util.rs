// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::NaiveDateTime;
use rand_core::OsRng;
use serde::Serializer;

/// `NaiveDateTime` → `"YYYY-MM-DD HH:MM:SS"`（前端直接展示）。
pub fn ser_dt<S: Serializer>(v: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&v.format("%Y-%m-%d %H:%M:%S").to_string())
}

pub fn ser_opt_dt<S: Serializer>(v: &Option<NaiveDateTime>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(dt) => s.serialize_str(&dt.format("%Y-%m-%d %H:%M:%S").to_string()),
        None => s.serialize_none(),
    }
}

/// 小写十六进制摘要（SHA-256 / HMAC 的展示形态）。不引 hex 依赖：只在这里写一次。
pub fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// 当前本地时间（naive）。
pub fn now() -> NaiveDateTime {
    chrono::Local::now().naive_local()
}

/// 当前 unix 秒（限流模块的时间基准）。
/// 秒级时间戳。**保持 u64**：唯一的消费者是 security-rust 的限流（throttle 的
/// API 全是 u64）。它不是一个 id，别跟着 i64 化。
pub fn now_unix() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

/// argon2id 哈希（随机盐）。
pub fn hash_password(plain: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .expect("argon2 哈希失败")
        .to_string()
}

pub fn verify_password(plain: &str, hashed: &str) -> bool {
    match PasswordHash::new(hashed) {
        Ok(h) => Argon2::default().verify_password(plain.as_bytes(), &h).is_ok(),
        Err(_) => false,
    }
}

/// argon2id 哈希的异步壳：CPU / 内存都压到阻塞线程池，别占 tokio worker。
pub async fn hash_password_async(plain: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || hash_password(&plain))
        .await
        .map_err(|e| format!("argon2 哈希任务失败: {e}"))
}

/// argon2id 校验的异步壳；任务本身失败（panic / 取消）按「密码不对」处理。
pub async fn verify_password_async(plain: String, hashed: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&plain, &hashed)).await.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_roundtrip() {
        let h = hash_password("admin123");
        assert!(h.starts_with("$argon2"), "unexpected hash: {h}");
        assert!(verify_password("admin123", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("admin123", "not-a-hash"));
    }

    #[tokio::test]
    async fn async_hash_roundtrip() {
        let h = hash_password_async("admin123".into()).await.expect("异步哈希失败");
        assert!(h.starts_with("$argon2"), "unexpected hash: {h}");
        assert!(verify_password_async("admin123".into(), h.clone()).await);
        assert!(!verify_password_async("wrong".into(), h).await);
    }
}
