// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;

/// 用户名不存在时也要付一次 argon2 校验，抹平「用户存在与否」的响应时差
/// （A2：提前返回会跳过 argon2，快出来的那截就是用户名枚举的时序侧信道）。
/// 这是用本项目 `util::hash_password("bee-admin-dummy")` 生成的一次性 argon2id
/// 哈希，只烧 CPU，结果丢弃。
pub(crate) const DUMMY_PASSWORD_HASH: &str =
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

pub(crate) fn user_agent(headers: &HeaderMap) -> String {
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
pub(crate) async fn find_or_create_device(
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
        // (admin_id, UA) 没有唯一键：并发登录可能留下两行。取**最早**那一行（id ASC）
        // 是确定的：新行出现不会让已发的 token 改绑到别的行。
        .order_by("id ASC")
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
pub(crate) fn lock_secs(lock_minutes: i64) -> u64 {
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
pub(crate) fn user_key(username: &str) -> String {
    format!("user:{}", username.chars().take(64).collect::<String>())
}

/// IP 维度的桶名。无 X-Real-IP / X-Forwarded-For（直连）时 `client_ip()` 返回
/// "unknown"：所有人会挤进同一个桶，按它限流等于误伤全体用户 → 这一维直接不做。
pub(crate) fn ip_key(ip: &str) -> Option<String> {
    (ip != "unknown").then(|| format!("ip:{ip}"))
}

/// 一次登录失败计两个维度：账号（谁被猜）+ IP（从哪猜的）。
/// 计数写失败不影响响应 —— 限流是纵深防御，不是主认证闸门。
pub(crate) fn record_login_failure(state: &AppState, username: &str, ip: &str) {
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
            None, // 账号锁定与公告邮件无关（邮件只认 notice.published）
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
pub(crate) const CAPTCHA_GLOBAL_KEY: &str = "captcha:all";

/// 全局桶的窗口内额度（次/分钟）。
///
/// 为什么不按建议的 300：实测一张验证码在插件存储里落 **2 条**记录（载荷 + 图），
/// 合计约 26–54 KB（JSON 13–27 KB，图与载荷各一份）。300/min × TTL 300s ≈ 1500 张
/// ≈ 40–80 MB，正好顶满插件的 64 MiB 预算 —— 顶满后写入 fail-closed，登录就炸。
/// 150/min 减半到 ~20–40 MB（预算的三到六成），既留出余量，又比单 IP 的 60/min
/// 高一个量级：正常用户打不满，刷子被压住。
pub(crate) const CAPTCHA_GLOBAL_MAX: u32 = 150;

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
pub(crate) fn captcha_global_throttle() -> &'static Arc<Throttle<MemoryThrottleStore>> {
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
pub(crate) static CAPTCHA_CONSUME: Mutex<()> = Mutex::new(());

/// 校验验证码凭据；通过即消费（同一 key 不能再过第二次）。
///
/// 失败一律 400 + `auth.captcha_failed`（不是 401/403）：「没带」「答错」「过期」
/// 「key 不存在」不给不同信号 —— 它们对客户端是同一件事（重来一次），区分开只会
/// 给爆破方提供反馈。
///
/// **不计入登录失败计数**（这里不调 `record_login_failure`）：验证码没过等于密码压根
/// 没提交，计进去只会让手抖的用户把自己锁死，或者攻击者拿别人的用户名刷垃圾验证码
/// 就把别人锁死（不需要任何密码猜测）。限流挡的是密码爆破，验证码是它前面另加的一道。
pub(crate) fn verify_captcha(
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
