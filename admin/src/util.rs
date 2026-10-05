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

/// 当前本地时间（naive）。
pub fn now() -> NaiveDateTime {
    chrono::Local::now().naive_local()
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
}
