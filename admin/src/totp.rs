// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! TOTP 动态验证码（RFC 6238；底层 HOTP = RFC 4226）：HMAC-SHA1 + base32 密钥 +
//! otpauth URL + 二维码 SVG。**纯函数**，不碰库、不打日志 —— 密钥只进不出
//! （密文落库的加解密在调用方 `api::totp`，见 `crypto`）。
//!
//! 参数取 RFC 与验证器 App 的公开默认：SHA1 / 6 位 / 30 秒。位数**不是**可配项：
//! otpauth URL 里虽写着 digits，但改了有的 App 不认。
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use sha1::Sha1;
use sha2::{Digest, Sha256};

type HmacSha1 = Hmac<Sha1>;

/// 步长（秒）
pub const STEP: u64 = 30;
/// 验证码位数
pub const DIGITS: u32 = 6;
/// 密钥字节数：160 位是 RFC 4226 §4 的推荐值，base32 后正好 32 个字符。
const SECRET_BYTES: usize = 20;
/// 校验窗口：当前步前后各 1 步（客户端时钟差 ±30 秒以内都能通过）
pub const WINDOW: i64 = 1;

/// 备用码枚数与长度（F4-1）：10 枚、每枚 10 个字符。
pub const BACKUP_COUNT: usize = 10;
pub const BACKUP_LEN: usize = 10;
/// 备用码字母表（32 个）：大写字母去掉 I/O、数字去掉 0/1 —— 只剩肉眼不易混淆的字符。
/// 32 是 2 的幂，按字节取模没有偏差。
const BACKUP_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// 备用码哈希的域分隔串：把 `encrypt_key` 派生成专用 pepper，避免同一把密钥在
/// 两处用途之间的输出可互相代入（Ruling F-2：pepper 由现有 `encrypt_key` 派生）。
const BACKUP_PEPPER_LABEL: &[u8] = b"bee-admin/totp-backup-code/v1:";

/// HOTP（RFC 4226 §5.3）：计数器 8 字节大端 → HMAC-SHA1 → 动态截断 → 末 `digits` 位。
/// 截断后是 31 位，用 u32 装得下，不用大整数。
pub fn hotp(secret: &[u8], counter: u64, digits: u32) -> String {
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC 支持任意长度密钥");
    mac.update(&counter.to_be_bytes());
    let d = mac.finalize().into_bytes();
    // §5.4 动态截断：末字节低 4 位当偏移，从那儿取 4 字节并抹掉最高位（避免符号）
    let off = (d[19] & 0x0f) as usize;
    let bin = u32::from_be_bytes([d[off] & 0x7f, d[off + 1], d[off + 2], d[off + 3]]);
    format!("{:0width$}", bin % 10u32.pow(digits), width = digits as usize)
}

/// 校验：与当前步、前后各 [`WINDOW`] 步比，命中就回传**步号**（批次 E S1：调用方拿它
/// 记「这枚码用过了」；只关心过不过的用 `.is_some()`）。这里不做恒定时间比较 —— 验证码
/// 是用户自己输进来的公开材料，比对耗时差不构成侧信道（泄漏的只是「这一步有没有这个码」）。
/// 从高到低试，多步同时命中（6 位码撞车，概率 1e-6 量级）时取最大的 —— 记的步号宁可
/// 超前不落后，落后会让已经作废的码重新可用。
pub fn verify_step(secret: &[u8], code: &str, unix: u64, window: i64) -> Option<i64> {
    let code = code.trim();
    if code.is_empty() || secret.is_empty() {
        return None;
    }
    let counter = (unix / STEP) as i64;
    (-window..=window)
        .rev()
        .filter_map(|i| {
            let c = counter + i;
            (c >= 0).then_some(c)
        })
        .find(|&c| hotp(secret, c as u64, DIGITS) == code)
}

/// 随机密钥（20 字节）。与口令哈希同源：rand_core 的 OsRng。
pub fn random_secret() -> Vec<u8> {
    let mut buf = [0u8; SECRET_BYTES];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut buf);
    buf.to_vec()
}

/// 一枚备用码（10 个字符）。`random_bytes % 32` 无偏（256 是 32 的整数倍）。
pub fn backup_code() -> String {
    let mut buf = [0u8; BACKUP_LEN];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut buf);
    buf.iter().map(|b| BACKUP_ALPHABET[(b & 31) as usize] as char).collect()
}

/// 一批备用码（10 枚，批内去重）。50 位随机空间下撞车概率可忽略，去重只是防御。
pub fn backup_codes() -> Vec<String> {
    let mut out = Vec::with_capacity(BACKUP_COUNT);
    while out.len() < BACKUP_COUNT {
        let code = backup_code();
        if !out.contains(&code) {
            out.push(code);
        }
    }
    out
}

/// 用户输入的备用码归一化：去空白与连字符、转大写（展示与复制都容错）。
pub fn normalize_backup_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase()
}

/// 备用码哈希：SHA-256(域分隔 ‖ encrypt_key ‖ 归一化后的码)，小写 hex。
/// `pepper` 传 `[app] encrypt_key` 原串 —— 派生是确定性的，重启后仍能验旧码。
pub fn backup_code_hash(pepper: &str, code: &str) -> String {
    let mut h = Sha256::new();
    h.update(BACKUP_PEPPER_LABEL);
    h.update(pepper.as_bytes());
    h.update(b":");
    h.update(normalize_backup_code(code).as_bytes());
    crate::util::hex_lower(&h.finalize())
}

/// 登录分流用：6 位纯数字 = TOTP，其余（含空串）走备用码分支。
pub fn is_totp_code(code: &str) -> bool {
    let code = code.trim();
    code.len() == DIGITS as usize && code.bytes().all(|b| b.is_ascii_digit())
}

/// base32 字母表（RFC 4648 §6）
const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// base32 编码（RFC 4648 §6，**不带 `=` 填充**）：验证器 App 与 otpauth URL 都收这种。
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut acc, mut bits) = (0u64, 0u32);
    for &b in data {
        acc = (acc << 8) | b as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(B32[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// base32 解码：宽容大小写、空格与 `=` 填充（别处导出的密钥常带这些），
/// 认不出的字符直接跳过 —— 反正最后是当 HMAC 的密钥字节用，垃圾进就是垃圾码。
pub fn base32_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u64, 0u32);
    for c in s.chars() {
        let v = match c.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u64 - 'A' as u64,
            c @ '2'..='7' => c as u64 - '2' as u64 + 26,
            _ => continue,
        };
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// otpauth URL（验证器 App 扫码/手输认的标准格式）：
/// `otpauth://totp/{issuer}:{account}?secret=…&issuer=…&algorithm=SHA1&digits=6&period=30`
/// 标签与查询值各自百分号编码，`:` 是标签里的分隔符（不编码）。
pub fn otpauth_url(issuer: &str, account: &str, secret_b32: &str) -> String {
    format!(
        "otpauth://totp/{}:{}?secret={secret_b32}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP}",
        enc(issuer),
        enc(account),
        enc(issuer),
    )
}

/// URI 里要编码的字节集：非字母数字，再放行 RFC 3986 的 unreserved `-._~`
/// （不编码它们只是为了 URL 好看，不影响正确性）。
const URI: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, URI).to_string()
}

/// 二维码 SVG（qrcode 只画黑白格，尺寸给个下限保证手机能扫）。
/// 入参是含密钥的 URL：**只回响应，不落盘不打日志**。
pub fn qr_svg(text: &str) -> Result<String, qrcode::types::QrError> {
    let code = qrcode::QrCode::new(text.as_bytes())?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4226 Appendix D 的官方向量（逐字抄自 RFC）：密钥 ASCII
    /// "12345678901234567890"，计数器 0-9，6 位码。
    #[test]
    fn rfc4226_appendix_d_vectors() {
        let secret = b"12345678901234567890";
        let want = [
            "755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583",
            "399871", "520489",
        ];
        for (counter, want) in want.iter().enumerate() {
            assert_eq!(&hotp(secret, counter as u64, 6), want, "counter = {counter}");
        }
    }

    /// RFC 6238 Appendix B 的官方向量（SHA1 行，**8 位**码 —— 手抄成 6 位会
    /// 永远对不上，RFC 原文就写着 `TOTP generation uses 8 digits`）。
    /// 密钥同为 "12345678901234567890"。
    #[test]
    fn rfc6238_appendix_b_vectors() {
        let secret = b"12345678901234567890";
        let cases: [(u64, &str); 6] = [
            (59, "94287082"),
            (1111111109, "07081804"),
            (1111111111, "14050471"),
            (1234567890, "89005924"),
            (2000000000, "69279037"),
            (20000000000, "65353130"),
        ];
        for (t, want) in cases {
            // TOTP 就是 HOTP，计数器 = floor(unix / 30)
            assert_eq!(hotp(secret, t / STEP, 8), want, "T = {t}");
        }
    }

    #[test]
    fn base32_matches_rfc4648_and_roundtrips() {
        // RFC 4648 §10 的向量（另一个方向用不了：它带 `=` 填充，我们编码不带）
        assert_eq!(base32_encode(b"f"), "MY");
        assert_eq!(base32_encode(b"fo"), "MZXQ");
        assert_eq!(base32_encode(b"foo"), "MZXW6");
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
        // 20 字节密钥 → 32 字符（5 位一组，不填充）
        let key = b"12345678901234567890";
        let b32 = base32_encode(key);
        assert_eq!(b32.len(), 32);
        assert_eq!(base32_decode(&b32), key, "编解码往返");
        // 宽容：小写 / 空格 / 填充
        assert_eq!(base32_decode("mzxw6 ytboi"), base32_decode("MZXW6YTBOI"));
        assert_eq!(base32_decode("gezdgnbvgy3tqojqgezdgnbvgy3tqojq"), key.to_vec());
    }

    #[test]
    fn verify_accepts_only_the_window() {
        let secret = b"12345678901234567890";
        let t = 1_111_111_111u64;
        let now = hotp(secret, t / STEP, DIGITS);
        let ok = |code: &str, at: u64| verify_step(secret, code, at, 1).is_some();
        assert!(ok(&now, t), "当前步");
        assert!(ok(&now, t + STEP), "晚一步（时钟慢）");
        assert!(ok(&now, t - STEP), "早一步（时钟快）");
        assert!(!ok(&now, t + 2 * STEP), "两步之外不行");
        assert!(!ok("000000", t) || now == "000000");
        assert!(!ok("", t), "空码直接拒（别把空串当合法码）");
        assert!(
            verify_step(b"", "123456", t, 1).is_none(),
            "空密钥直接拒（没绑就不该能过）"
        );
    }

    /// S1：校验要回传**命中的步号**（登录侧的一次性账本就认这个值）。
    #[test]
    fn verify_step_reports_the_matched_counter() {
        let secret = b"12345678901234567890";
        let t = 1_111_111_111u64;
        let cur = t / STEP;
        for off in [-1i64, 0, 1] {
            let c = (cur as i64 + off) as u64;
            let code = hotp(secret, c, DIGITS);
            assert_eq!(verify_step(secret, &code, t, 1), Some(c as i64), "偏移 {off} 的码");
        }
        assert_eq!(verify_step(secret, "000000", t, 1).filter(|&c| c == cur as i64), None);
        assert_eq!(verify_step(secret, "", t, 1), None, "空码");
        assert_eq!(verify_step(b"", "123456", t, 1), None, "空密钥");
    }

    #[test]
    fn random_secret_and_url_shape() {
        let s = random_secret();
        assert_eq!(s.len(), 20);
        assert_ne!(random_secret(), s, "随机源要真的随机");
        let b32 = base32_encode(&s);
        let url = otpauth_url("Bee Admin", "admin@x.com", &b32);
        assert!(url.starts_with("otpauth://totp/Bee%20Admin:admin%40x.com?"), "{url}");
        assert!(url.contains(&format!("secret={b32}")), "{url}");
        assert!(url.contains("issuer=Bee%20Admin"), "{url}");
        assert!(url.contains("digits=6&period=30"), "{url}");
    }

    #[test]
    fn qr_svg_is_an_svg() {
        let svg = qr_svg("otpauth://totp/x").unwrap();
        // 渲染器会先写一行 XML 声明，所以看开头是 `<svg` 会误判（实测过）
        assert!(svg.contains("<svg xmlns"), "{svg:.60}");
        assert!(svg.ends_with("</svg>"), "{svg:.60}");
    }

    #[test]
    fn backup_codes_are_10_unique_ten_char_codes_without_confusables() {
        let codes = backup_codes();
        assert_eq!(codes.len(), BACKUP_COUNT);
        for c in &codes {
            assert_eq!(c.chars().count(), BACKUP_LEN, "{c}");
            assert!(
                c.bytes().all(|b| BACKUP_ALPHABET.contains(&b)),
                "只用去掉 0/O/1/I 的字母表: {c}"
            );
        }
        let mut uniq = codes.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), BACKUP_COUNT, "批内不能有重复: {codes:?}");
        assert_ne!(backup_codes(), codes, "随机源要真的随机");
    }

    #[test]
    fn backup_code_normalization_and_hash_vector() {
        // 展示大写；用户小写、带连字符或空格也能验
        assert_eq!(normalize_backup_code(" abcde-23456 "), "ABCDE23456");
        // 已知向量（独立用 python hashlib 算的固定值）：域分隔 + pepper + 码
        assert_eq!(
            backup_code_hash("pepper-test-key", "ABCDE23456"),
            "bb3aef85de4d6d7f50ba57debcd83d0ba82d4dc055b72e2517002ef40bf6f5fa"
        );
        assert_eq!(
            backup_code_hash("pepper-test-key", "abcde 23456"),
            backup_code_hash("pepper-test-key", "ABCDE23456"),
            "归一化后同一枚码必须是同一个哈希"
        );
        assert_ne!(
            backup_code_hash("other-pepper", "ABCDE23456"),
            backup_code_hash("pepper-test-key", "ABCDE23456"),
            "换 pepper 必须换哈希"
        );
    }

    #[test]
    fn only_six_ascii_digits_are_totp_codes() {
        assert!(is_totp_code("123456"));
        assert!(is_totp_code(" 123456 "));
        for bad in ["", "12345", "1234567", "12345a", "ABCDE23456", "１２３４５６"] {
            assert!(!is_totp_code(bad), "{bad:?}");
        }
    }
}
