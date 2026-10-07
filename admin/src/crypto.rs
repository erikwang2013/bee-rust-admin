// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! `admin.email` / `admin.phone` 的落库加密（encryptable-rust，AES-256-GCM）。
//!
//! 规矩只有两条，读写点各一条：
//! - **写**：任何把明文写进这两列的地方先过 [`write`]（create / update_profile / seed /
//!   存量迁移）。模型上不做 serde 钩子 —— 那样密钥得藏在全局状态里，比在出口显式调一次难查。
//! - **读**：JSON / CSV 出口前过 [`plain`]（[`plain_json`] 是列表/详情共用的批量版）。
//!   只解真密文：`is_encrypted` 为假的存量明文原样返回，所以迁移前后都能读。
//!
//! 这两列从不作为查询条件，所以走随机 nonce 的 AEAD 就够了（确定性加密那条路用不上）。
//! 加密只保护落库形态：库被拖走时 email/phone 不可读，**不防「密钥也在同一台机器上」**。
use crate::error::ApiError;
use encryptable::config::ArrayConfig;
use encryptable::guard::Guard;
use serde_json::Value;

/// email / phone 列的宽度（模型 `#[bee(sql_type = "VARCHAR(255)")]`）。
/// 实测密文长度 = `4 × ceil((明文字节数 + 30) / 3)`（1 版本 + 12 nonce + 1 长度 + 密文 + 16 tag，
/// 再 base64）：空串 40、11 字节 56、16 字节 64、128 字节 212，255 都放得下 —— email/phone
/// 业务上只有 ASCII，够用；但校验按**字符数**算，128 个多字节字符（UTF-8 384 字节）密文 552
/// 会撑爆。所以写之前卡一道，给 400/告警而不是写库时才收一个 500「Data too long」。
const COL_MAX: usize = 255;

/// 从 `[app] encrypt_key` 建守卫。密钥格式：`base64:<32 字节>` / 64 位 hex / 32 字节
/// 字面量（见插件文档）。**配置错了就在这里炸**，不拖到第一次读写。
pub fn build_guard(key: &str) -> Result<Guard, String> {
    Guard::new(&ArrayConfig::new(key)).map_err(|e| format!("[app] encrypt_key 不可用: {e}"))
}

/// 明文 → 密文。`Guard::encrypt` 对「已经是本格式密文」的输入原样返回（会真的试解一次），
/// 所以重复调用不会双重加密。
pub fn write(guard: &Guard, plain: &str) -> Result<String, String> {
    let cipher = guard.encrypt(plain).map_err(|e| format!("加密失败: {e}"))?;
    if cipher.len() > COL_MAX {
        return Err(format!(
            "加密后 {} 个字符，超出列宽 {COL_MAX}（明文含多字节字符？）",
            cipher.len()
        ));
    }
    Ok(cipher)
}

/// 库里读出的值 → 明文。只解真密文：存量明文原样返回（迁移没跑完也读得出）。
pub fn plain(guard: &Guard, value: &str) -> Result<String, String> {
    if !guard.is_encrypted(value) {
        return Ok(value.to_owned());
    }
    guard.decrypt_text(value).map_err(|e| format!("解密失败: {e}"))
}

/// 就地修正序列化后的 JSON：email / phone 换成明文（列表 / 详情 / 导出共用）。
pub fn plain_json(guard: &Guard, v: &mut Value) -> Result<(), ApiError> {
    for field in ["email", "phone"] {
        let Some(raw) = v.get(field).and_then(Value::as_str) else { continue };
        let plain = plain(guard, raw).map_err(ApiError::internal)?;
        v[field] = Value::String(plain);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与 conf/app.conf.test 同源的固定密钥（32 字节）。
    const KEY: &str = "base64:YmVlLWFkbWluLXRlc3Qta2V5LTMyLWJ5dGVzLW9rISE=";

    #[test]
    fn round_trip_and_empty_string() {
        let g = build_guard(KEY).unwrap();
        let c = write(&g, "a@b.c").unwrap();
        assert!(c.len() > "a@b.c".len(), "密文要长于明文: {c}");
        assert_eq!(plain(&g, &c).unwrap(), "a@b.c");

        // 空串（业务上允许空邮箱/空手机号）也要能存能读
        let e = write(&g, "").unwrap();
        assert_eq!(plain(&g, &e).unwrap(), "");
    }

    #[test]
    fn legacy_plaintext_reads_as_is() {
        let g = build_guard(KEY).unwrap();
        assert_eq!(plain(&g, "13800000000").unwrap(), "13800000000");
        assert_eq!(plain(&g, "").unwrap(), "");
    }

    #[test]
    fn write_is_not_double_encrypting() {
        let g = build_guard(KEY).unwrap();
        let c = write(&g, "x@y.z").unwrap();
        assert_eq!(write(&g, &c).unwrap(), c, "已加密的输入原样返回（含迁移重跑）");
    }

    #[test]
    fn ciphertext_that_does_not_fit_the_column_is_rejected() {
        let g = build_guard(KEY).unwrap();
        // 128 个汉字 = 384 字节 → 密文 552 > 255：必须在写库前拦掉
        assert!(write(&g, &"邮".repeat(128)).is_err());
        assert!(write(&g, &"a".repeat(128)).is_ok(), "128 字节 ASCII 密文 212，放得下");
    }

    #[test]
    fn plain_json_only_touches_the_two_fields() {
        let g = build_guard(KEY).unwrap();
        let mut v = serde_json::json!({
            "email": write(&g, "a@b.c").unwrap(),
            "phone": write(&g, "13800000000").unwrap(),
            "username": "admin",
        });
        plain_json(&g, &mut v).unwrap();
        assert_eq!(v["email"], "a@b.c");
        assert_eq!(v["phone"], "13800000000");
        assert_eq!(v["username"], "admin");
    }

    #[test]
    fn bad_config_fails_here_not_at_first_use() {
        assert!(build_guard("").is_err(), "空密钥不能静默通过");
        assert!(build_guard("base64:change-me-32-random-bytes-here").is_err());
    }
}
