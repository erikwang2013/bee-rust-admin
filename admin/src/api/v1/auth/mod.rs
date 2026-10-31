// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::auth::{Auth, expire_secs, sign_token};
use crate::config::AppConfig;
use crate::crypto;
use crate::hid;
use crate::error::{ApiError, AppJson, CAPTCHA_FAILED, ok};
use crate::models::{Admin, AdminLoginDevice, LoginLog, Menu};
use crate::state::AppState;
use crate::util::{now, now_unix};
use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use bee_orm::Model;
use poster::captcha::Answer;
use security_rust::throttle::{
    MemoryThrottleStore, Throttle, ThrottleConfig, ThrottleDecision, ThrottleOutcome,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use crate::relations::RelationsExt;

mod guard;
mod login;
mod session;

pub use self::guard::*;
pub use self::login::*;
pub use self::session::*;

#[derive(Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
    /// 验证码 key（`[auth] captcha = true` 时必填）。字段形状与 poster-rust 校验端点一致。
    pub captcha_key: Option<String>,
    /// 验证码答案：poster-rust `Answer` 的 serde 表示（外部标签）：
    /// `{"Slider": 173.0}` / `{"Rotate": 37.2}` / `{"Click": [[x, y], …]}`。
    pub captcha_answer: Option<Answer>,
    /// 动态验证码（项 7）：该账号绑定了 TOTP 时必填（`err = auth.totp_required` 提示前端补上）
    pub code: Option<String>,
}

#[derive(Deserialize)]
pub struct ChangePasswordBody {
    pub old_password: String,
    pub new_password: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    // 顶层 import 里没有它了（`IntoResponse` 只被这里的断言用）
    use axum::response::IntoResponse;

    /// 合法 key：poster 2.x 起校验形态（32 位十六进制，即 `generate_key()` 的产物）。
    /// 生产 key 由插件生成、天然合法；夹具必须造合法的，否则 2.x 直接判无效。
    const KEY: &str = "0123456789abcdef0123456789abcdef";
    /// 形态合法但存储里不存在（「key 不存在」那一档用）。
    const KEY_ABSENT: &str = "ffffffffffffffffffffffffffffffff";
    /// 限流用例的第二、第三个 key（都要合法且互不相同）。
    const KEY2: &str = "22222222222222222222222222222222";
    const KEY3: &str = "33333333333333333333333333333333";

    /// 造一个用内存存储的守卫，并**直接塞一份已知答案的载荷**：验证码的答案只存在
    /// 服务端（生成时要画图），测试里没有「解题」一说，只能这样拿到可校验的凭据。
    /// 载荷字段与插件 `finish()` 写的一致（`type` + 各类型专属字段）。
    fn test_guard(payload: &str) -> (poster::Guard, Arc<poster::storage::MemoryStorage>) {
        test_guard_opts(payload, false)
    }

    /// 同上，但轨迹校验按 `[auth] captcha_trajectory` 的开/关各来一份。
    fn test_guard_opts(
        payload: &str,
        trajectory_enabled: bool,
    ) -> (poster::Guard, Arc<poster::storage::MemoryStorage>) {
        use poster::captcha::CaptchaManager;
        use poster::storage::MemoryStorage;
        let mem = Arc::new(MemoryStorage::new());
        put_key(&mem, KEY, payload);
        // 限流窗口拉长到 1 小时：默认 60s 的固定窗口在「连着打 31 发」时可能正好跨窗，
        // 计数清零会让「额度耗尽」的断言偶发失败。这里要测的是我们这层的消费与分桶，
        // 不是插件限流器本身的窗口滚动。
        let mut config = poster::PosterConfig::default();
        config.captcha.rate_limit.window_secs = 3600;
        config.captcha.trajectory.enabled = trajectory_enabled;
        let manager =
            CaptchaManager::with_config_and_storage(Arc::new(config), mem.clone());
        (poster::Guard::new(Arc::new(manager)), mem)
    }

    /// 塞一份已知答案的载荷。**键名必须带 `captcha:` 前缀**：poster 2.x 起载荷存在
    /// `captcha:{key}` 的命名空间里（`payload_key()`，见 2.0 的 `captcha/mod.rs`：
    /// 把用户提交的 key 圈进自己的命名空间，防拼出限流键等别的子系统的键）。
    /// 写裸 key 会「找不到载荷」→ 一律 400。前缀是插件的内部约定，夹具跟着它走。
    fn put_key(mem: &poster::storage::MemoryStorage, key: &str, payload: &str) {
        use poster::storage::Storage;
        mem.set(
            &format!("captcha:{key}"),
            payload.as_bytes(),
            std::time::Duration::from_secs(300),
        )
        .expect("内存存储写入不会失败");
    }

    /// 通过 → 该 key 立刻作废（一次性，不可重放）；答错 → 400 而不是 401；
    /// 缺 key / 缺答案 / key 不存在 → 同样 400（不区分原因）。
    #[test]
    fn captcha_verify_is_one_shot_and_400_on_failure() {
        let (guard, _mem) = test_guard(r#"{"type":"slider","x":100.0}"#);

        // 容差内（±4px）判过
        verify_captcha(&guard, Some(KEY), Some(Answer::Slider(103.0)), "1.2.3.4")
            .expect("容差内应通过");
        // 一次性：同一 key 再来一次必失败（校验通过时载荷已被删掉）
        let err = verify_captcha(&guard, Some(KEY), Some(Answer::Slider(100.0)), "1.2.3.4")
            .unwrap_err();
        assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST, "重放要 400");

        // 答错
        let (guard, _mem) = test_guard(r#"{"type":"slider","x":100.0}"#);
        let err = verify_captcha(&guard, Some(KEY), Some(Answer::Slider(200.0)), "1.2.3.4")
            .unwrap_err();
        assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST);

        // 没带凭据 / key 不存在：与本项目「登录失败」的 401 分开，统一 400
        for (key, answer) in [
            (None, Some(Answer::Slider(100.0))),
            (Some(KEY), None),
            (Some(KEY_ABSENT), Some(Answer::Slider(100.0))),
        ] {
            let err = verify_captcha(&guard, key, answer, "1.2.3.4").unwrap_err();
            assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST, "{key:?}");
        }

        // 类型不符（载荷是 slider，答案按 click 交）也只是一次普通失败
        let (guard, _mem) = test_guard(r#"{"type":"slider","x":100.0}"#);
        assert!(verify_captcha(&guard, Some(KEY), Some(Answer::Click(vec![(1.0, 1.0)])), "1.2.3.4").is_err());
    }

    /// 轨迹校验（`[auth] captcha_trajectory = true` → 插件的 `captcha.trajectory.enabled`）。
    /// 三组断言都拿**正确的 x** 去交，能不能过只差在轨迹上 —— 否则「谁都没过」也可能
    /// 只是 x 答错了，测不出开关真的接上了。
    #[test]
    fn captcha_trajectory_gate() {
        const PAYLOAD: &str = r#"{"type":"slider","x":100.0}"#;
        // 人手轨迹，照插件自己的用例（先停一下再冲过去，线性度约 0.77 < 阈值 0.99）
        let human = || vec![(0.0, 0.0, 0.0), (5.0, 0.0, 150.0), (6.0, 1.0, 300.0), (30.0, 0.0, 450.0)];

        // a) 只交裸数值（老前端 / 脚本）：开关打开后必须失败
        let (guard, _mem) = test_guard_opts(PAYLOAD, true);
        let err = verify_captcha(&guard, Some(KEY), Some(Answer::Slider(100.0)), "1.2.3.4")
            .unwrap_err();
        assert_eq!(
            err.into_response().status(),
            StatusCode::BAD_REQUEST,
            "开轨迹后裸数值不认"
        );

        // b) 带合法轨迹（5 点 / 1000ms / 明显非线性，线性度约 0.77）→ 通过。
        //    这根轨迹**从 JSON 反序列化**：下面的字面量与前端 `captcha.test.ts` 里逐字
        //    断言的请求体是同一份（「按下停一下再拖」的采样），「两边的形状对得上」
        //    由它钉死，不靠注释约定。
        const WIRE: &str = r#"{"SliderWithTrail":{"x":100,"trail":[[0,0,0],[0,0,300],[4,0,450],[9,2,600],[50,0,1000]],"duration_ms":1000}}"#;
        let answer: Answer = serde_json::from_str(WIRE).expect("前端提交的形状必须能反序列化");
        let (guard, _mem) = test_guard_opts(PAYLOAD, true);
        verify_captcha(&guard, Some(KEY), Some(answer), "1.2.3.4")
            .expect("合法轨迹 + 正确 x 应通过");

        // c) 点太少 / 太快 → 失败（x 仍是对的，判定的只能是轨迹）
        for (trail, duration_ms, why) in [
            (vec![(0.0, 0.0, 0.0), (10.0, 0.0, 200.0), (30.0, 0.0, 400.0)], 400u64, "只有 3 点"),
            (human(), 100u64, "100ms 甩过去的"),
        ] {
            let (guard, _mem) = test_guard_opts(PAYLOAD, true);
            let err = verify_captcha(
                &guard,
                Some(KEY),
                Some(Answer::SliderWithTrail { x: 100.0, trail, duration_ms }),
                "1.2.3.4",
            )
            .unwrap_err();
            assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST, "{why}");
        }
    }

    /// 限流按身份分桶（身份由登录接口传的 `client_ip()` 派生）：一个 IP 打满额度后
    /// 连正确答案也拦（限流在判定之前），但**不影响别的 IP** —— 若身份是常量，
    /// 所有人会挤在一个桶里互相误杀，这个断言就是那道闸。
    #[test]
    fn captcha_rate_limit_is_per_identity() {
        const PAYLOAD: &str = r#"{"type":"slider","x":100.0}"#;
        let (guard, mem) = test_guard(PAYLOAD);
        for _ in 0..30 {
            let _ = verify_captcha(&guard, Some(KEY), Some(Answer::Slider(0.0)), "9.9.9.9");
        }
        // 9.9.9.9 的额度已耗尽：换个新 key 交正确答案也不放行
        put_key(&mem, KEY2, PAYLOAD);
        assert!(
            verify_captcha(&guard, Some(KEY2), Some(Answer::Slider(100.0)), "9.9.9.9").is_err(),
            "同一身份超窗口上限后必须关闭"
        );
        // 同一个存储、同一个窗口，另一个身份照常通过
        put_key(&mem, KEY3, PAYLOAD);
        assert!(verify_captcha(&guard, Some(KEY3), Some(Answer::Slider(100.0)), "1.1.1.1").is_ok());
    }

    /// 全局桶（`/captcha/new` 的第二道闸）：必须是**同一个**进程级实例（static，
    /// 不是每次调用新建一个 —— 那样等于没限），额度耗尽后走 `Allow { remaining: 0 }`
    /// 而不是封禁（ban_secs = 0，窗口滑过自动恢复，不误伤后面的人）。
    #[test]
    fn captcha_global_throttle_caps_the_process() {
        let t = captcha_global_throttle();
        assert!(Arc::ptr_eq(t, captcha_global_throttle()), "必须是同一个 static 桶");
        let now = now_unix();
        for _ in 0..CAPTCHA_GLOBAL_MAX {
            t.record_failure(CAPTCHA_GLOBAL_KEY, now).expect("内存存储计数不会失败");
        }
        assert_eq!(
            t.check(CAPTCHA_GLOBAL_KEY, now),
            ThrottleDecision::Allow { remaining: 0 },
            "额度耗尽后拒绝本请求（与单 IP 桶同一套判定）"
        );
        // 额度按 key 计：全局性来自「对 `/captcha/new` 永远只用一个 key」
        assert_eq!(
            t.check("captcha:1.2.3.4", now),
            ThrottleDecision::Allow { remaining: CAPTCHA_GLOBAL_MAX },
            "别的 key 不受全局桶影响"
        );
    }

    /// 上面那个额度是**算出来的**，算式的输入就在这儿钉住：一张验证码落 2 条记录、
    /// 单条 ~13–27 KB。将来插件换了更大的图 / 更贵的载荷，这条会先红，提醒重算
    /// `CAPTCHA_GLOBAL_MAX`（64 MiB 预算 ÷ TTL 300s ÷ 单张占用）。
    #[test]
    fn captcha_footprint_matches_the_global_budget_arithmetic() {
        use poster::captcha::CaptchaManager;
        use poster::storage::MemoryStorage;
        let mem = Arc::new(MemoryStorage::new());
        let manager =
            CaptchaManager::with_config_and_storage(Arc::new(poster::PosterConfig::default()), mem.clone());
        let guard = poster::Guard::from_manager(Arc::new(manager)).expect("构造守卫");
        for i in 1..=3 {
            let v = guard.create_json(None).expect("生成验证码");
            let len = serde_json::to_string(&v).unwrap().len();
            assert_eq!(mem.len(), i * 2, "一张验证码 = 载荷 + 图两条记录");
            assert!(len < 64 * 1024, "第 {i} 张 {len} 字节，超出额度算式用的上限");
        }
    }

    /// 假哈希必须是**可解析**的 argon2 串：解析失败时 `verify_password` 会立刻返回
    /// false（不跑 KDF），A2 的抹平效果就静默失效了。
    #[test]
    fn dummy_hash_is_a_real_argon2_hash() {
        let h = argon2::PasswordHash::new(DUMMY_PASSWORD_HASH)
            .expect("占位哈希必须是合法 argon2id PHC 串");
        assert_eq!(h.algorithm.as_str(), "argon2id");
        // 且真的会跑一遍校验（不 panic、不提前返回）
        assert!(!crate::util::verify_password("not-the-dummy-plaintext", DUMMY_PASSWORD_HASH));
    }

    #[test]
    fn client_ip_priority_and_truncation() {
        let mut h = HeaderMap::new();
        // 无任何头 → unknown
        assert_eq!(client_ip(&h), "unknown");
        // X-Forwarded-For 取第一个
        let _ = h.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());
        assert_eq!(client_ip(&h), "1.2.3.4");
        // X-Real-IP 优先
        let _ = h.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        assert_eq!(client_ip(&h), "9.9.9.9");
        // 客户端可控的超长头截到 45 字符（IPv6 文本上限），不会写库超长
        let _ = h.insert("x-real-ip", "x".repeat(300).parse().unwrap());
        assert_eq!(client_ip(&h).len(), 45);
    }

    /// 钉住一个真实存在过的 bug：窗口/封禁时长曾经因为 `.min(u64::MAX as i64)`
    /// （= 对任何正值都取 min(-1)）而后 `as u64` 变成 `u64::MAX`，
    /// 等于「失败 5 次封 584 亿年」，而前端提示的仍是「请 10 分钟后再试」。
    #[test]
    fn lock_secs_is_minutes_not_infinite() {
        assert_eq!(lock_secs(10), 600, "默认 lock_minutes=10 → 600 秒");
        assert_eq!(lock_secs(1), 60);
        assert_eq!(lock_secs(1440), 86_400, "一天");
        // 极端值走饱和：不回绕、不 panic
        assert_eq!(lock_secs(i64::MAX), i64::MAX as u64);
        assert_ne!(lock_secs(10), u64::MAX, "绝不能是「永不过期」");
    }
}
