// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 项 11 / 12：`/api/v1/version`（构建期注入版本号）与 `/metrics`（Prometheus 文本）。
//!
//! 起真实进程 + 真库（bee_admin_test），需要 `BEE_ADMIN_DB_DSN`。
//! 版本号断言用 `env!("BEE_ADMIN_VERSION")`：测试二进制与服务二进制由**同一次编译**
//! 产生，build.rs 注入的值一致 —— 断言成写死的 "dev" 会在 `BRA_VERSION=… cargo test`
//! 时假红。
mod common;

use common::{dsn, reset_db, start_server};

const HEALTH: &str = "/api/v1/health";

/// 从 Prometheus 文本里取一条样本的值：`<前缀> <数字>`。
fn sample(text: &str, prefix: &str) -> f64 {
    text.lines()
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("指标里没有 `{prefix}`\n{text}"))
        .rsplit_once(' ')
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or_else(|| panic!("`{prefix}` 后面的值不是数字\n{text}"))
}

#[tokio::test]
async fn version_and_metrics() {
    let Some(dsn) = dsn() else { return };
    reset_db(&dsn).await;
    let (_server, base) = start_server().await;
    let c = reqwest::Client::new();

    // ── 项 11：版本接口。免登录（与 /health 同级口径），回 {"version": …} ──
    let r = c.get(format!("{base}/api/v1/version")).send().await.unwrap();
    assert_eq!(r.status(), 200, "版本接口不带 token 也要通");
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(
        body["version"].as_str().unwrap_or("<缺 version 字段>"),
        env!("BEE_ADMIN_VERSION"),
        "版本号来自构建期注入（build.rs ← BRA_VERSION）"
    );

    // ── 项 12：指标端点。文本格式、有启动时间 ──
    let r = c.get(format!("{base}/metrics")).send().await.unwrap();
    assert_eq!(r.status(), 200, "运维端点不鉴权（服务只监听 127.0.0.1）");
    let ct = r.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ct.starts_with("text/plain"), "Prometheus 抓取端按文本格式解析: {ct}");
    let text = r.text().await.unwrap();
    assert!(text.contains("# TYPE process_start_time_seconds gauge\n"), "{text}");
    assert!(
        sample(&text, "process_start_time_seconds ") > 1_600_000_000.0,
        "启动时间该是 Unix 秒:\n{text}"
    );

    // 打几个请求：3 个 2xx + 1 个 405（路径在、方法不对）+ 1 个 404（路径不存在）
    for _ in 0..3 {
        let r = c.get(format!("{base}{HEALTH}")).send().await.unwrap();
        assert_eq!(r.status(), 200);
    }
    let r = c.post(format!("{base}{HEALTH}")).send().await.unwrap();
    assert_eq!(r.status(), 405, "路径存在但方法不对");
    let r = c.get(format!("{base}/api/v1/nope")).send().await.unwrap();
    assert_eq!(r.status(), 404);

    let text = c.get(format!("{base}/metrics")).send().await.unwrap().text().await.unwrap();

    // 状态码大类：启动探活 + 上面 3 次都在 2xx；405/404 都归 4xx
    assert!(
        sample(&text, "http_requests_total{status=\"2xx\"} ") >= 4.0,
        "启动探活 1 次 + 健康检查 3 次:\n{text}"
    );
    assert!(
        sample(&text, "http_requests_total{status=\"4xx\"} ") >= 2.0,
        "405 + 404 各一次:\n{text}"
    );

    // 路由模板口径：per-path 计数 = 直方图 count = +Inf 桶
    let count = sample(&text, &format!("http_requests_by_path_total{{path=\"{HEALTH}\"}} "));
    assert!(count >= 4.0, "health 至少 4 次（含启动探活）:\n{text}");
    assert_eq!(
        sample(&text, &format!("http_request_duration_seconds_count{{path=\"{HEALTH}\"}} ")),
        count,
        "同一次观测两边都要记"
    );
    assert_eq!(
        sample(&text, &format!("http_request_duration_seconds_bucket{{le=\"+Inf\",path=\"{HEALTH}\"}} ")),
        count,
        "+Inf 桶 = 观测总数"
    );

    // 高基数护栏：未匹配的原始路径**不能**进指标，归到固定标签 <unmatched>
    assert!(
        !text.contains("/api/v1/nope"),
        "未匹配路径必须归一化，否则扫描器随便打 URL 就能撑爆指标表:\n{text}"
    );
    assert!(
        text.contains("http_requests_by_path_total{path=\"<unmatched>\"} "),
        "兜底路由要归到 <unmatched>:\n{text}"
    );
    // 指标端点不进 apidoc：`metrics::endpoint` 没有 `#[apidoc::…]` 注解，
    // 文档站只收注解过的处理器（/apidoc/api.json 的清单由 linkme 注册项拼出）。
}
