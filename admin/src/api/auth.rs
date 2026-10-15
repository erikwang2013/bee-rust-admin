// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::auth::{Auth, expire_secs, sign_token};
use crate::config::AppConfig;
use crate::crypto;
use crate::hid;
use crate::error::{ApiError, AppJson, CAPTCHA_FAILED, ok};
use crate::models::{Admin, AdminLoginDevice, LoginLog, Menu};
use crate::state::AppState;
use crate::util::{hash_password, now, now_unix, verify_password};
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

/// 用户名不存在时也要付一次 argon2 校验，抹平「用户存在与否」的响应时差
/// （A2：提前返回会跳过 argon2，快出来的那截就是用户名枚举的时序侧信道）。
/// 这是用本项目 `util::hash_password("bee-admin-dummy")` 生成的一次性 argon2id
/// 哈希，只烧 CPU，结果丢弃。
const DUMMY_PASSWORD_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$dhEKLDVWOgNaVRGlknZrTw$SoG1K6HuZZmwffQCWSyxO55B9KpBGRKrsxC8w+CVhTs";

/// 客户端 IP：nginx 透传的 X-Real-IP → X-Forwarded-For 首个 → unknown。
/// 头部由客户端可控，截到 IPv6 文本最长 45 字符，避免写库超长报错。
pub fn client_ip(headers: &HeaderMap) -> String {
    let raw = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| {
            headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    raw.chars().take(45).collect()
}

fn user_agent(headers: &HeaderMap) -> String {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(255)
        .collect()
}

pub async fn write_login_log(
    state: &AppState,
    admin_id: i64,
    username: &str,
    headers: &HeaderMap,
    status: i8,
    msg: &str,
) {
    // 发号失败 = 这条记录留不下（登录本身的成败不受影响）
    let Ok(id) = state.next_id() else {
        tracing::error!("生成登录记录 id 失败，跳过本次留档");
        return;
    };
    let log = LoginLog {
        id,
        admin_id,
        username: username.chars().take(64).collect(),
        ip: client_ip(headers),
        user_agent: user_agent(headers),
        status,
        msg: msg.chars().take(255).collect(),
        created_at: now(),
    };
    if let Err(e) = log.insert(&state.db).await {
        // 登录记录写失败不能影响登录本身
        tracing::error!("写登录记录失败: {e}");
    }
}

/// 找到或新建一条设备行（项 7）。身份是 (admin_id, UA)：同 UA 复用一行、顺手更新 IP
/// 与活跃时间；被踢过的行（status=0）不复用 —— 复用它等于踢完立刻又能用同一条，
/// 列表上也看不出踢过。
///
/// shortcut: (admin_id, UA) 上没有唯一键，同一账号同 UA 同时登两次会建出两行；
/// 列表/踢出都按行来，多一行只是多一条设备记录。要收口就加 `(admin_id, user_agent,
/// status)` 唯一键 + 冲突重试。
async fn find_or_create_device(
    state: &AppState,
    admin_id: i64,
    ua: &str,
    ip: &str,
) -> Result<AdminLoginDevice, ApiError> {
    let now = now();
    let found = AdminLoginDevice::query()
        .filter_eq("admin_id", admin_id)
        .map_err(ApiError::from)?
        .filter_eq("user_agent", ua)
        .map_err(ApiError::from)?
        .filter_eq("status", 1)
        .map_err(ApiError::from)?
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;
    if let Some(mut d) = found.into_iter().next() {
        d.ip = ip.to_string();
        d.last_active_at = now;
        d.update(&state.db).await.map_err(ApiError::from)?;
        return Ok(d);
    }
    let d = AdminLoginDevice {
        id: state.next_id()?,
        admin_id,
        user_agent: ua.to_string(),
        ip: ip.to_string(),
        status: 1,
        created_at: now,
        last_active_at: now,
    };
    d.insert(&state.db).await.map_err(ApiError::from)?;
    Ok(d)
}

/// 限流窗口与封禁时长（秒）。调用方已保证 `lock_minutes > 0`。
///
/// **这里曾经有一个真实存在过的 bug**：原先写成
/// `lock_minutes.saturating_mul(60).min(u64::MAX as i64) as u64` ——
/// `u64::MAX as i64` 是 **-1**，对任何正值取 `min` 都得 -1，再 `as u64` 变成
/// `u64::MAX`：等于把窗口与封禁设成约 584 亿年。前端拿到的仍是
/// `throttled(lock_minutes)` 那句「请 N 分钟后再试」，两边对不上。
/// 内存桶进程重启即清，所以这个 bug 一直没被暴露。
/// 迁移时化简掉了那次多余的 clamp，顺带修好；下面的单测钉住它。
fn lock_secs(lock_minutes: i64) -> u64 {
    lock_minutes.saturating_mul(60) as u64
}

/// 登录限流闸门：`max_fail` 次窗口内失败即封 `lock_minutes` 分钟，账号与 IP 同一阈值。
/// 两个维度同阈值是有意的：IP 桶比账号桶宽松时，攻击者能用「不触发自己上限」的
/// 失败量反复把别人的账号刷封，而自己不被拦。
/// `max_fail <= 0` 或 `lock_minutes <= 0` = 关闭（沿用既有配置约定）。
pub fn login_throttle(cfg: &AppConfig) -> Option<Arc<Throttle<MemoryThrottleStore>>> {
    if cfg.max_fail <= 0 || cfg.lock_minutes <= 0 {
        return None;
    }
    let secs = lock_secs(cfg.lock_minutes);
    Some(Arc::new(Throttle::new(
        MemoryThrottleStore::new(),
        ThrottleConfig {
            threshold: cfg.max_fail.min(u32::MAX as i64) as u32,
            window_secs: secs,
            ban_secs: secs,
        },
    )))
}

/// 账号维度的桶名。截断到与 `login_log.username` 同样的 64 字符：
/// 库要求 key 有界（否则人人换一个超长用户名就能撑爆内存）。
fn user_key(username: &str) -> String {
    format!("user:{}", username.chars().take(64).collect::<String>())
}

/// IP 维度的桶名。无 X-Real-IP / X-Forwarded-For（直连）时 `client_ip()` 返回
/// "unknown"：所有人会挤进同一个桶，按它限流等于误伤全体用户 → 这一维直接不做。
fn ip_key(ip: &str) -> Option<String> {
    (ip != "unknown").then(|| format!("ip:{ip}"))
}

/// 一次登录失败计两个维度：账号（谁被猜）+ IP（从哪猜的）。
/// 计数写失败不影响响应 —— 限流是纵深防御，不是主认证闸门。
fn record_login_failure(state: &AppState, username: &str, ip: &str) {
    let Some(throttle) = &state.throttle else { return };
    let now = now_unix();
    // 达到阈值的判定由库在 record_failure 内部做（返回 Banned），这里不用再算。
    // 「刚好刷爆桶」的这一次顺手发一个通知事件（项 6）——只发**账号**维度：
    // IP 维度爆掉是「这个来源被限流」，不是某个账号被锁，拿它发「账号锁定」是误导。
    let uk = user_key(username);
    match throttle.record_failure(&uk, now) {
        Ok(ThrottleOutcome::Banned { until }) => crate::notify::emit(
            state,
            crate::notify::EVENT_ACCOUNT_LOCKED,
            json!({ "username": username, "ip": ip, "until": until }),
        ),
        Ok(_) => {}
        Err(e) => tracing::error!("登录限流计数失败 ({uk}): {e}"),
    }
    if let Some(ik) = ip_key(ip) {
        if let Err(e) = throttle.record_failure(&ik, now) {
            tracing::error!("登录限流计数失败 ({ik}): {e}");
        }
    }
}

/// 验证码**生成**接口（`/captcha/new`，公开、每次要渲一张 PNG）的限流闸门。
///
/// 单独一个实例、单独一套阈值，不复用登录那套：登录闸门是「N 次失败即封 M 分钟」，
/// 拿它来限「刷新验证码」语义不对，还会把正常用户刷进封禁。
/// 这里 `ban_secs = 0` —— 只数数、不封人：窗口内额度耗尽由调用方按
/// `Allow { remaining: 0 }` 拒绝（与登录闸门同一套判定），窗口滑过去自动恢复。
///
/// 为什么必须有：插件默认的进程内存储**只在读到某个 key 时才顺手清它的过期条目**
/// （`storage/memory.rs` 里没有清扫任务），而每次生成都是一个新随机 key —— 没人去读
/// 的 key 会永久占着内存。没有这道闸门，任何人都能靠刷 `/captcha/new` 让进程内存
/// 线性上涨（PNG + base64，每条几十 KB）。
///
/// ponytail: 60 次/分钟是拍的：正常用户一分钟开不了 60 次登录页，而刷子被压到
/// 60 条/分钟（按 TTL 300s 算单 IP 稳态 ≤ 300 条）。要调就调这两个数。
pub fn captcha_create_throttle() -> Arc<Throttle<MemoryThrottleStore>> {
    Arc::new(Throttle::new(
        MemoryThrottleStore::new(),
        ThrottleConfig {
            threshold: 60,
            window_secs: 60,
            ban_secs: 0,
        },
    ))
}

/// 全局桶的键。只有这一条，见 [`captcha_global_throttle`]。
const CAPTCHA_GLOBAL_KEY: &str = "captcha:all";

/// 全局桶的窗口内额度（次/分钟）。
///
/// 为什么不按建议的 300：实测一张验证码在插件存储里落 **2 条**记录（载荷 + 图），
/// 合计约 26–54 KB（JSON 13–27 KB，图与载荷各一份）。300/min × TTL 300s ≈ 1500 张
/// ≈ 40–80 MB，正好顶满插件的 64 MiB 预算 —— 顶满后写入 fail-closed，登录就炸。
/// 150/min 减半到 ~20–40 MB（预算的三到六成），既留出余量，又比单 IP 的 60/min
/// 高一个量级：正常用户打不满，刷子被压住。
const CAPTCHA_GLOBAL_MAX: u32 = 150;

/// `/captcha/new` 的**全局**桶：不限来源，把整个进程的验证码生成速率钉在一个上界上。
///
/// **这是速率兜底，不是精准配额。** 单 IP 那个桶（[`captcha_create_throttle`]）按
/// `client_ip()` 分桶，而 `client_ip()` 读的是 **X-Real-IP / X-Forwarded-For 请求头**：
/// 换个头就是一个新桶，单 IP 上限对分布式（或直接换头的）来源形同虚设。而每次生成都往
/// 插件的进程内存储里塞两份记录（PNG + 载荷，几十 KB），64 MiB 预算（poster-rust
/// `storage/memory.rs` 的 `DEFAULT_BUDGET`）填满后写入一律失败（fail-closed）→
/// `/captcha/new` 全 500，而登录强制要验证码 = 全站登不进去。这道闸不吃请求头，
/// 多来源刷子也绕不过：速率被压到 ~150/min，64 MiB 的填充时间从「不限速的几十秒」
/// 拉到小时级（TTL 300s 内最多几百条在库）。
///
/// 为什么不并入 `state.captcha_throttle` 加个 key：`Throttle` 的阈值是**实例级**的，
/// 全局额度要比单 IP 高一个量级（单 IP 打不满它），同实例就得同阈值，只能单独一个。
/// 也因为它只有一条键（不像单 IP 桶的键随 IP 增长），不需要进 `main.rs` 的定时 purge。
///
/// ponytail: 单进程量。多实例部署下每个进程各算各的 —— 那种部署本来就要换掉
/// poster 的进程内存储（见 `state.rs` 的说明），这道闸到那时该挪到网关。
fn captcha_global_throttle() -> &'static Arc<Throttle<MemoryThrottleStore>> {
    static GLOBAL: OnceLock<Arc<Throttle<MemoryThrottleStore>>> = OnceLock::new();
    GLOBAL.get_or_init(|| {
        Arc::new(Throttle::new(
            MemoryThrottleStore::new(),
            ThrottleConfig {
                threshold: CAPTCHA_GLOBAL_MAX,
                window_secs: 60,
                ban_secs: 0,
            },
        ))
    })
}

/// 验证码「校验 + 消费」的进程内互斥锁。
///
/// 插件的 `verify_as` 是「读载荷 → 原子自增计数 → 通过后删 key」：并发下同一个 key 的
/// 多发请求都能读到载荷，各自自增（不超过 max_attempts 的都放行），等于一次解出能挤进
/// 最多 3 次密码猜测。锁把这一段串起来，让「用过即废」在并发下也成立。
/// 锁内只有同步代码（内存存储，无 await），代价是一次哈希表查询。
///
/// ponytail: 只在单进程内有效。多实例部署要换插件的 Redis 存储，那才是真正跨进程的消费。
static CAPTCHA_CONSUME: Mutex<()> = Mutex::new(());

/// 校验验证码凭据；通过即消费（同一 key 不能再过第二次）。
///
/// 失败一律 400 + `auth.captcha_failed`（不是 401/403）：「没带」「答错」「过期」
/// 「key 不存在」不给不同信号 —— 它们对客户端是同一件事（重来一次），区分开只会
/// 给爆破方提供反馈。
///
/// **不计入登录失败计数**（这里不调 `record_login_failure`）：验证码没过等于密码压根
/// 没提交，计进去只会让手抖的用户把自己锁死，或者攻击者拿别人的用户名刷垃圾验证码
/// 就把别人锁死（不需要任何密码猜测）。限流挡的是密码爆破，验证码是它前面另加的一道。
fn verify_captcha(
    guard: &poster::Guard,
    key: Option<&str>,
    answer: Option<Answer>,
    identity: &str,
) -> Result<(), ApiError> {
    let (Some(key), Some(answer)) = (key, answer) else {
        return Err(ApiError::BadRequest(CAPTCHA_FAILED.into()));
    };
    let _consume = CAPTCHA_CONSUME.lock().unwrap_or_else(|e| e.into_inner());
    match guard.verify_as(key, answer, identity) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ApiError::BadRequest(CAPTCHA_FAILED.into())),
        // 存储坏掉时失败关闭（拿不准就不放行），与插件自身的口径一致。
        // 500 而不是 400：这是服务端故障而非「你答错了」，运维要能从状态码上看见。
        Err(e) => Err(ApiError::internal(format!("验证码存储故障: {e}"))),
    }
}

/// 登录页验证码（`GET /api/v1/captcha/new`，公开）。
///
/// 响应 `data` 的形状 = poster-rust 的 `CaptchaResult`：`{key, image, type, extra}`；
/// `image` 是 PNG 的 data URI，`type` ∈ click|rotate|slider（默认 `random` → 三选一），
/// `extra` 随类型变（click: `texts[{text,order}]`、slider: `puzzle,puzzle_w,puzzle_h`、
/// rotate: `{}`）。**答案只在服务端的存储里**，前端拿不到 —— 这正是它挡机器人的地方。
///
/// `[auth] captcha = false` 时 `data` 为 `null`：开关只在这一处判断，前端不需要另开
/// 一个「查配置」接口，也不会出现「前端显示了验证码而后端根本不校验」的错位。
#[apidoc::title("生成登录验证码")]
#[apidoc::desc("**公开接口**。返回 poster-rust 的验证码载荷（key + 图形数据），登录时随登录体一起提交校验；按 IP 限流（60 秒窗口）")]
#[apidoc::url("/api/v1/captcha/new")]
#[apidoc::method("GET")]
#[apidoc::tag("认证")]
#[apidoc::response_status("200")]
#[apidoc::response_status("429")]
#[apidoc::returned(name = "data", ty = "object", desc = "插件原始载荷：key + 图形数据（字段随验证码类型
（点击 / 旋转 / 滑块，每次随机切）变化）")]
pub async fn captcha_new(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    if !state.cfg.captcha {
        return Ok(ok(Value::Null));
    }
    // 生成口的限流（见 `captcha_create_throttle` 里「刷它能让内存涨」的说明）。
    // `unknown`（无 X-Real-IP 也无 XFF，多为直连）**也照限**：这里与 `ip_key()` 的取舍
    // 相反 —— 那边不限 unknown 只是少一维防护，这边不限就是内存随便涨。代价是 NAT
    // 后面的一群人共用一个桶，而这个桶最多让人「这一分钟内换不了验证码」，不封人。
    let key = format!("captcha:{}", client_ip(&headers));
    let now = now_unix();
    let throttle = &state.captcha_throttle;
    if matches!(
        throttle.check(&key, now),
        ThrottleDecision::Banned { .. } | ThrottleDecision::Allow { remaining: 0 }
    ) {
        return Err(ApiError::throttled(1)); // 窗口 60s，所以报 1 分钟
    }
    // 全局桶（见 `captcha_global_throttle` 的说明）：单 IP 桶按请求头分桶、换个头就绕，
    // 这道不吃请求头，是内存增长的速率兜底
    let global = captcha_global_throttle();
    if matches!(
        global.check(CAPTCHA_GLOBAL_KEY, now),
        ThrottleDecision::Banned { .. } | ThrottleDecision::Allow { remaining: 0 }
    ) {
        return Err(ApiError::throttled(1));
    }
    // 计数写失败不影响响应 —— 限流是纵深防御，不是主认证闸门
    if let Err(e) = throttle.record_failure(&key, now) {
        tracing::error!("验证码生成限流计数失败 ({key}): {e}");
    }
    if let Err(e) = global.record_failure(CAPTCHA_GLOBAL_KEY, now) {
        tracing::error!("验证码生成全局限流计数失败: {e}");
    }
    let value = state
        .captcha
        .create_json(None)
        .map_err(|e| ApiError::internal(format!("生成验证码失败: {e}")))?;
    Ok(ok(value))
}

#[apidoc::title("登录")]
#[apidoc::desc("用户名 + 密码登录，返回 JWT 与当前用户信息。受失败锁定（[auth] max_fail / lock_minutes）与图形验证码（[auth] captcha）保护：过不去就不比密码，响应不泄露密码是否正确")]
#[apidoc::url("/api/v1/auth/login")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::param(name = "username", ty = "string", required, desc = "用户名", mock = "admin")]
#[apidoc::param(name = "password", ty = "string", required, desc = "密码（明文提交，务必走 HTTPS）", mock = "admin123")]
#[apidoc::param(name = "captcha_key", ty = "string", desc = "[auth] captcha = true 时必填：/api/v1/captcha/new 返回的 key")]
#[apidoc::param(name = "captcha_answer", ty = "object", desc = "验证码答案，形状同 poster-rust Answer（Slider 滑动 / Rotate 旋转 / Click 点选）")]
#[apidoc::param(name = "code", ty = "string", desc = "动态验证码（该账号绑定 TOTP 后必填；缺了回 err = auth.totp_required，前端据此显示输入框后带码重试）", mock = "123456")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("429")]
#[apidoc::returned(name = "data", ty = "object", desc = "登录结果", children = [
            {name = "token", ty = "string", required, desc = "JWT，后续请求放 Authorization: Bearer <token>"},
            {name = "expires_in", ty = "int", required, desc = "有效期（秒）"},
            {name = "user", ty = "object", required, desc = "当前用户", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "username", ty = "string", required, desc = "用户名"},
            {name = "nickname", ty = "string", desc = "昵称"},
            {name = "avatar", ty = "string", desc = "头像地址（无头像为空串）"},
            {name = "email", ty = "string", desc = "邮箱（库里密文，出口明文）"},
            {name = "phone", ty = "string", desc = "手机号（同上）"},
            {name = "sex", ty = "int", desc = "性别：0 未知 / 1 男 / 2 女"},
            {name = "is_super", ty = "bool", desc = "是否超级管理员"},
            {name = "dept_id", ty = "string", desc = "部门短串（0 = 无部门）"},
            {name = "totp_enabled", ty = "bool", desc = "是否已绑定动态验证码（项 7）"},
        ]},
        ])]
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    AppJson(body): AppJson<LoginBody>,
) -> Result<Json<Value>, ApiError> {
    if body.username.trim().is_empty() || body.password.is_empty() {
        return Err(ApiError::BadRequest("用户名和密码不能为空".into()));
    }
    let username = body.username.trim();
    let ip = client_ip(&headers);

    // 限流闸门在验密之前：过不去就不比对密码，响应也不泄露密码是否正确
    if let Some(throttle) = &state.throttle {
        let now = now_unix();
        let uk = user_key(username);
        let ik = ip_key(&ip);
        let keys: Vec<&str> = std::iter::once(uk.as_str()).chain(ik.as_deref()).collect();
        let decision = throttle.check_any(&keys, now);
        // `Allow { remaining: 0 }` 是「额度已耗尽、本请求应被拒绝」，不是「还能再试一次」
        if matches!(
            decision,
            ThrottleDecision::Banned { .. } | ThrottleDecision::Allow { remaining: 0 }
        ) {
            write_login_log(&state, 0, username, &headers, 0, "请求过于频繁").await;
            // 封禁时长 = 窗口 = lock_minutes，额度耗尽最晚也是等窗口滑完
            return Err(ApiError::throttled(state.cfg.lock_minutes));
        }
        // 存储故障不 fail-closed（库有意如此）：限流是纵深防御，主认证闸门在后面
        if decision == ThrottleDecision::Unavailable {
            tracing::warn!("登录限流存储不可用，本次放行");
        }
    }

    // 验证码闸门（[auth] captcha）放在限流之后、查库之前：
    // - 限流之后：封禁中的请求仍是 429（既有行为不变），验证码不能变成绕过封禁的路子；
    // - 查库之前：没过验证码就不碰库、不烧 argon2，也就没有用户名侧信道可用。
    if state.cfg.captcha {
        let identity = client_ip(&headers);
        if let Err(e) = verify_captcha(
            &state.captcha,
            body.captcha_key.as_deref(),
            body.captcha_answer.clone(),
            &identity,
        ) {
            write_login_log(&state, 0, username, &headers, 0, CAPTCHA_FAILED).await;
            return Err(e);
        }
    }

    let admin = Admin::query()
        .filter_eq("username", username)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?;

    let Some(admin) = admin else {
        // 与下面「密码错误」分支付同样的 argon2 代价（结果丢弃），
        // 否则两条路径的耗时差就是用户名枚举的侧信道
        let _ = verify_password(&body.password, DUMMY_PASSWORD_HASH);
        write_login_log(&state, 0, username, &headers, 0, "用户不存在").await;
        record_login_failure(&state, username, &ip);
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    };
    if !verify_password(&body.password, &admin.password) {
        write_login_log(&state, admin.id, &admin.username, &headers, 0, "密码错误").await;
        record_login_failure(&state, username, &ip);
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    }
    if admin.status != 1 {
        write_login_log(&state, admin.id, &admin.username, &headers, 0, "账号已禁用").await;
        record_login_failure(&state, username, &ip);
        return Err(ApiError::BadRequest("账号已被禁用".into()));
    }

    // 动态验证码闸门（项 7）：绑定了才要。放在密码之后 —— 码是第二因子，
    // 先要码等于让不知道密码的人也能把这条路径走一遍。
    if admin.totp_enabled == 1 {
        let code = body.code.as_deref().unwrap_or("");
        if code.trim().is_empty() {
            // 客户端没带码（老前端/脚本）是协议没对齐，不是有人在猜密码：
            // 留档但**不计数** —— 否则一个没升级的前端能把用户刷进封禁。
            write_login_log(&state, admin.id, &admin.username, &headers, 0, "需要动态验证码").await;
            return Err(ApiError::BadRequest(crate::error::TOTP_REQUIRED.into()));
        }
        if !crate::api::totp::verify_code(&state, &admin, code)? {
            write_login_log(&state, admin.id, &admin.username, &headers, 0, "动态验证码错误").await;
            record_login_failure(&state, username, &ip);
            return Err(ApiError::BadRequest(crate::error::BAD_TOTP.into()));
        }
    }

    // 设备行（项 7）：同账号同 UA 复用一条，token 里带上它的 id，之后能单独踢掉这台
    let device = find_or_create_device(&state, admin.id, &user_agent(&headers), &ip).await?;
    let (token, expires_in) = sign_token(
        &state.jwt,
        admin.id,
        admin.token_version,
        device.id,
        expire_secs(&state.cfg),
    )?;

    let mut updated = admin.clone();
    updated.last_login_at = Some(now());
    updated.last_login_ip = client_ip(&headers);
    updated.updated_at = now();
    updated.update(&state.db).await.map_err(ApiError::from)?;

    // 成功即清掉该账号的失败计数（修复①：4 次失败 + 1 次成功 + 1 次失败不该锁死）。
    // IP 桶**不清**：桶是共享的，攻击者拿自己账号登录一次就能替爆破者洗掉计数，
    // 封禁也会被顺手解除 —— 库的 record_success 只清计数不清封禁，正是为此。
    if let Some(throttle) = &state.throttle {
        if let Err(e) = throttle.record_success(&user_key(username)) {
            tracing::error!("登录成功后清零限流计数失败: {e}");
        }
    }

    write_login_log(&state, admin.id, &admin.username, &headers, 1, "登录成功").await;

    // 库里 email / phone 是密文，出去前解成明文（前端无感）
    let email = crypto::plain(&state.crypto, &admin.email).map_err(ApiError::internal)?;
    let phone = crypto::plain(&state.crypto, &admin.phone).map_err(ApiError::internal)?;

    Ok(ok(json!({
        "token": token,
        "expires_in": expires_in,
        "user": {
            "id": hid::enc(admin.id),
            "username": admin.username,
            "nickname": admin.nickname,
            "avatar": admin.avatar,
            "email": email,
            "phone": phone,
            "sex": admin.sex,
            "is_super": admin.is_super == 1,
            "dept_id": hid::enc(admin.dept_id),
            "totp_enabled": admin.totp_enabled == 1,
        }
    })))
}

#[apidoc::title("退出登录")]
#[apidoc::desc("当前设备的 token 立即失效（token_version 自增）。需要登录")]
#[apidoc::url("/api/v1/auth/logout")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn logout(
    State(state): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let mut admin = auth.admin.clone();
    admin.token_version += 1; // 当前 token 立即失效
    admin.updated_at = now();
    admin.update(&state.db).await.map_err(ApiError::from)?;
    write_login_log(&state, admin.id, &admin.username, &headers, 1, "退出登录").await;
    Ok(ok(Value::Null))
}

/// 退出其他设备（A3）：token_version 自增让所有旧 token 作废，然后给当前设备
/// 换发一枚新版本的 token —— 用旧版本签的当前设备 token 也一起失效了，
/// 不回新就得把发起者自己也踢下线。前端契约：新 token 在 `data.token`。
#[apidoc::title("退出其他设备")]
#[apidoc::desc("所有旧 token 一起作废，并给当前设备换发新 token（不回新 token 会把发起者自己也踢下线）。需要登录")]
#[apidoc::url("/api/v1/auth/logout-others")]
#[apidoc::method("POST")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "新 token", children = [
            {name = "token", ty = "string", required, desc = "换发后的 JWT"},
            {name = "expires_in", ty = "int", required, desc = "有效期（秒）"},
        ])]
pub async fn logout_others(
    State(state): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let mut admin = auth.admin.clone();
    admin.token_version += 1;
    admin.updated_at = now();
    admin.update(&state.db).await.map_err(ApiError::from)?;

    // 换发的 token 仍挂在**当前这台设备**上：不然一杯新 token 就把本机从设备列表里摘掉
    let (token, expires_in) = sign_token(
        &state.jwt,
        admin.id,
        admin.token_version,
        auth.device_id,
        expire_secs(&state.cfg),
    )?;
    write_login_log(&state, admin.id, &admin.username, &headers, 1, "退出其他设备").await;
    Ok(ok(json!({ "token": token, "expires_in": expires_in })))
}

#[apidoc::title("当前用户信息")]
#[apidoc::desc("个人中心回显：用户资料 + 角色标识 + 权限码集合（email / phone 出口解密为明文）。需要登录")]
#[apidoc::url("/api/v1/auth/profile")]
#[apidoc::method("GET")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "用户 + 角色 + 权限", children = [
            {name = "user", ty = "object", required, desc = "当前用户", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "username", ty = "string", required, desc = "用户名"},
            {name = "nickname", ty = "string", desc = "昵称"},
            {name = "avatar", ty = "string", desc = "头像地址（无头像为空串）"},
            {name = "email", ty = "string", desc = "邮箱（库里密文，出口明文）"},
            {name = "phone", ty = "string", desc = "手机号（同上）"},
            {name = "sex", ty = "int", desc = "性别：0 未知 / 1 男 / 2 女"},
            {name = "is_super", ty = "bool", desc = "是否超级管理员"},
            {name = "dept_id", ty = "string", desc = "部门短串（0 = 无部门）"},
            {name = "totp_enabled", ty = "bool", desc = "是否已绑定动态验证码（项 7）"},
        ]},
            {name = "roles", ty = "array", required, desc = "角色标识数组，如 [\"super\"]"},
            {name = "perms", ty = "array", required, desc = "权限码数组，如 [\"system:admin:list\"]"},
        ])]
pub async fn profile(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let mut perms: Vec<&String> = auth.perms.iter().collect();
    perms.sort();
    let roles: Vec<&str> = auth.roles.iter().map(|r| r.code.as_str()).collect();
    // 库里是密文，回给前端的是明文（个人中心要回显/编辑这几项，B7）
    let email = crypto::plain(&state.crypto, &auth.admin.email).map_err(ApiError::internal)?;
    let phone = crypto::plain(&state.crypto, &auth.admin.phone).map_err(ApiError::internal)?;
    Ok(ok(json!({
        "user": {
            "id": hid::enc(auth.admin.id),
            "username": auth.admin.username,
            "nickname": auth.admin.nickname,
            // 列里存的是插件的 saved_path（内部地址），对外一律换成可用的 URL
            "avatar": super::avatar::public_avatar(&auth.admin.avatar, auth.admin.id),
            "email": email,
            "phone": phone,
            "sex": auth.admin.sex,
            "is_super": auth.is_super,
            "dept_id": hid::enc(auth.admin.dept_id),
            // 个人中心要据此显示「绑定 / 解绑动态验证码」（项 7）
            "totp_enabled": auth.admin.totp_enabled == 1,
        },
        "roles": roles,
        "perms": perms,
    })))
}

/// 当前用户的菜单树（只含目录/菜单；超管全量，否则按角色勾选）。
#[apidoc::title("当前用户菜单树")]
#[apidoc::desc("只含目录 / 菜单（不含按钮）；超管拿全量，其余按已启用角色勾选（缺祖先节点时自动补全）。需要登录")]
#[apidoc::url("/api/v1/auth/menus")]
#[apidoc::method("GET")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "array", desc = "菜单树（顶层节点数组）", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "parent_id", ty = "string", required, desc = "父节点短串（0 = 顶层）"},
            {name = "name", ty = "string", required, desc = "菜单名"},
            {name = "path", ty = "string", desc = "前端路由路径"},
            {name = "icon", ty = "string", desc = "图标名"},
            {name = "children", ty = "array", desc = "子节点（同结构，递归）"},
        ])]
pub async fn menus(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let all = Menu::query()
        .filter_eq("status", 1)
        .map_err(ApiError::from)?
        .filter_eq("visible", 1)
        .map_err(ApiError::from)?
        .order_by("sort ASC, id ASC")
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;

    let allowed: Option<Vec<i64>> = if auth.is_super {
        None
    } else {
        let mut ids: Vec<i64> = Vec::new();
        for rid in auth.roles.iter().filter(|r| r.status == 1).map(|r| r.id) {
            ids.extend(
                state
                    .db
                    .get_relations("role_menu", ("role_id", rid), "menu_id")
                    .await
                    .map_err(ApiError::from)?,
            );
        }
        ids.sort_unstable();
        ids.dedup();
        // 客户端只提交子节点时（antd 半选不落库），补全祖先，否则拼不出树、侧边栏为空
        let parent_of: HashMap<i64, i64> = all.iter().map(|m| (m.id, m.parent_id)).collect();
        Some(crate::api::with_ancestors(&parent_of, &ids))
    };

    let visible: Vec<&Menu> = all
        .iter()
        .filter(|m| m.menu_type == "M" || m.menu_type == "C")
        .filter(|m| match &allowed {
            None => true,
            Some(ids) => ids.contains(&m.id),
        })
        .collect();

    fn build(parent: i64, nodes: &[&Menu]) -> Vec<Value> {
        nodes
            .iter()
            .filter(|m| m.parent_id == parent)
            .map(|m| {
                json!({
                    "id": hid::enc(m.id),
                    "parent_id": hid::enc(m.parent_id),
                    "name": m.name,
                    "path": m.path,
                    "icon": m.icon,
                    "children": build(m.id, nodes),
                })
            })
            .collect()
    }

    Ok(ok(build(0, &visible)))
}

#[derive(Deserialize)]
pub struct ProfileBody {
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
}

/// 改自己的资料。不碰密码/状态/角色，也不换 token（否则改个昵称就被踢下线）。
#[apidoc::title("修改个人资料")]
#[apidoc::desc("只改自己的昵称 / 邮箱 / 手机号：不碰密码、状态、角色，也不换 token（改个昵称不该被踢下线）")]
#[apidoc::url("/api/v1/auth/profile")]
#[apidoc::method("PUT")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "nickname", ty = "string", desc = "昵称，最长 64 字符")]
#[apidoc::param(name = "email", ty = "string", desc = "邮箱（落库加密），最长 128 字符")]
#[apidoc::param(name = "phone", ty = "string", desc = "手机号（落库加密），最长 20 字符")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn update_profile(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<ProfileBody>,
) -> Result<Json<Value>, ApiError> {
    crate::api::check_len("nickname", &body.nickname, 64)?;
    crate::api::check_len("email", &body.email, 128)?;
    crate::api::check_len("phone", &body.phone, 20)?;

    let mut admin = auth.admin.clone();
    admin.nickname = body.nickname;
    // 写库一律密文（`crate::crypto::write` 对已是密文的输入原样返回，重放安全）
    admin.email = crypto::write(&state.crypto, &body.email).map_err(ApiError::BadRequest)?;
    admin.phone = crypto::write(&state.crypto, &body.phone).map_err(ApiError::BadRequest)?;
    admin.updated_at = now();
    admin.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("修改自己的密码")]
#[apidoc::desc("校验原密码后换成新密码（argon2id）；成功后所有端下线（token_version 自增），需重新登录")]
#[apidoc::url("/api/v1/auth/password")]
#[apidoc::method("PUT")]
#[apidoc::tag("认证")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::param(name = "old_password", ty = "string", required, desc = "原密码")]
#[apidoc::param(name = "new_password", ty = "string", required, desc = "新密码，至少 8 位且含字母/数字/符号中至少两类")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
pub async fn change_password(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<ChangePasswordBody>,
) -> Result<Json<Value>, ApiError> {
    crate::api::admin::validate_password(&body.new_password, "新密码")?;
    if !verify_password(&body.old_password, &auth.admin.password) {
        return Err(ApiError::BadRequest("原密码错误".into()));
    }
    let mut admin = auth.admin.clone();
    admin.password = hash_password(&body.new_password);
    admin.token_version += 1; // 全端下线，需重新登录
    admin.updated_at = now();
    admin.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
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
        assert!(!verify_password("not-the-dummy-plaintext", DUMMY_PASSWORD_HASH));
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
