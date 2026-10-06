// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
mod api;
mod audit;
mod auth;
mod config;
mod datascope;
mod error;
mod models;
mod retention;
mod seed;
mod state;
mod util;

use axum::response::IntoResponse;
use config::AppConfig;
use state::AppState;
use tracing::Level;

async fn health() -> &'static str {
    "OK"
}

/// 路由级兜底只改 `/api/v1` 树内的响应形状；树外的路径保持 axum 原样
/// （树外将来可能挂静态资源/别的服务，不该替它们改响应）。
fn in_api(path: &str) -> bool {
    path == "/api/v1" || path.starts_with("/api/v1/")
}

/// 路由级 404（A1）：未匹配的路径也回 `{code,msg,data}` 信封。
async fn api_not_found(uri: axum::http::Uri) -> axum::response::Response {
    if in_api(uri.path()) {
        error::envelope(axum::http::StatusCode::NOT_FOUND, "接口不存在")
    } else {
        axum::http::StatusCode::NOT_FOUND.into_response()
    }
}

/// 路由级 405（A1）：路径存在但方法不对，同样回信封。
async fn api_method_not_allowed(uri: axum::http::Uri) -> axum::response::Response {
    if in_api(uri.path()) {
        error::envelope(axum::http::StatusCode::METHOD_NOT_ALLOWED, "方法不允许")
    } else {
        axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response()
    }
}

/// `log_level` 配置 → tracing 等级。非法值告警并回落 info（别因为一个拼写错误起不来）。
fn parse_level(raw: &str) -> Level {
    match raw.trim().to_ascii_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" | "warning" => Level::WARN,
        "error" => Level::ERROR,
        other => {
            eprintln!("配置 [app] log_level = {other} 无法识别，回落到 info");
            Level::INFO
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conf_path = std::env::var("BEE_ADMIN_CONF").unwrap_or_else(|_| "conf/app.conf".into());
    let cfg = AppConfig::load(&conf_path)?;

    // 先读配置再起日志，log_level 才真正生效（B1）；句柄要活到进程结束
    let _log = bee_rust::bee_logs::Logger::new().level(parse_level(&cfg.log_level)).init()?;
    tracing::info!("{} 启动，配置 {}", cfg.app_name, conf_path);

    let db = bee_orm::Db::connect(&cfg.db_dsn).await?;
    seed::migrate(&db).await?;
    seed::seed(&db, &cfg).await?;
    let throttle = api::auth::login_throttle(&cfg);
    let state = AppState { db, cfg: std::sync::Arc::new(cfg), throttle };
    let addr = state.cfg.http_addr.clone();

    // 内存限流的条目只在写路径顺手清窗口内的失败，桶本身（每个用户名/IP 一个）不会
    // 自己消失 —— 库要求按窗口量级定时 purge，否则 key 基数会一直吃内存。
    if let Some(throttle) = state.throttle.clone() {
        let period = std::time::Duration::from_secs((state.cfg.lock_minutes * 60) as u64);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(period);
            loop {
                tick.tick().await;
                match throttle.purge_expired(util::now_unix()) {
                    Ok(n) if n > 0 => tracing::debug!("清理过期限流条目 {n} 条"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("清理限流条目失败: {e}"),
                }
            }
        });
    }

    // 日志保留（A4）：启动先清一遍积压，然后每 24 小时一次。删的是慢增长的历史
    // 数据，日频足够；`retain_days = 0` 时不 spawn（永久保留）。
    if state.cfg.retain_days > 0 {
        let db = state.db.clone();
        let retain_days = state.cfg.retain_days;
        tokio::spawn(async move {
            loop {
                retention::purge_all(&db, retain_days).await;
                tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
            }
        });
    }

    let router = bee_rust::bee_router::Router::new()
        .ns("/api/v1", |ns| ns.get("/health", health))
        .ns("/api/v1/auth", |ns| {
            ns.post("/login", api::auth::login)
                .post("/logout", api::auth::logout)
                .post("/logout-others", api::auth::logout_others)
                .get("/profile", api::auth::profile)
                .put("/profile", api::auth::update_profile)
                .post("/avatar", api::auth::upload_avatar)
                .get("/menus", api::auth::menus)
                .put("/password", api::auth::change_password)
        })
        // 头像读取公开：<img> 标签带不了 Authorization 头
        .ns("/api/v1/avatar", |ns| ns.get("/{id}", api::auth::get_avatar))
        .ns("/api/v1/admins", |ns| {
            ns.get("", api::admin::list)
                .post("", api::admin::create)
                .get("/export", api::admin::export)
                .get("/{id}", api::admin::detail)
                .put("/{id}", api::admin::update)
                .delete("/{id}", api::admin::remove)
                .put("/{id}/status", api::admin::set_status)
                .put("/{id}/password", api::admin::reset_password)
                .put("/{id}/roles", api::admin::set_roles)
        })
        .ns("/api/v1/roles", |ns| {
            ns.get("", api::role::list)
                .post("", api::role::create)
                .get("/{id}", api::role::detail)
                .put("/{id}", api::role::update)
                .delete("/{id}", api::role::remove)
                .get("/{id}/menus", api::role::get_menus)
                .put("/{id}/menus", api::role::set_menus)
                .get("/{id}/depts", api::role::get_depts)
                .put("/{id}/depts", api::role::set_depts)
        })
        .ns("/api/v1/menus", |ns| {
            ns.get("/tree", api::menu::tree)
                .post("", api::menu::create)
                .put("/{id}", api::menu::update)
                .delete("/{id}", api::menu::remove)
        })
        .ns("/api/v1/depts", |ns| {
            ns.get("/tree", api::dept::tree)
                .post("", api::dept::create)
                .put("/{id}", api::dept::update)
                .delete("/{id}", api::dept::remove)
        })
        .ns("/api/v1/login-logs", |ns| {
            ns.get("", api::login_log::list)
                .delete("", api::login_log::clear)
                .get("/export", api::login_log::export)
        })
        .ns("/api/v1/audit-logs", |ns| {
            ns.get("", api::audit_log::list)
                .delete("", api::audit_log::clear)
                .get("/export", api::audit_log::export)
        })
        .with_state(state.clone())
        // 路由级兜底（A1）挂在 layer 之前：未匹配路径上的写操作照旧被审计留痕
        .fallback(api_not_found)
        .method_not_allowed_fallback(api_method_not_allowed)
        // 操作日志中间件（B5）：只记写操作，审计失败不影响业务
        .layer(axum::middleware::from_fn_with_state(state, audit::audit_mw));

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_parsing_and_fallback() {
        assert_eq!(parse_level("warn"), Level::WARN);
        assert_eq!(parse_level(" WARN "), Level::WARN);
        assert_eq!(parse_level("warning"), Level::WARN);
        assert_eq!(parse_level("debug"), Level::DEBUG);
        assert_eq!(parse_level("trace"), Level::TRACE);
        assert_eq!(parse_level("error"), Level::ERROR);
        // 非法值回落 info（并 eprintln 告警），不能让进程起不来
        assert_eq!(parse_level("verbose"), Level::INFO);
        assert_eq!(parse_level(""), Level::INFO);
    }

    /// 兜底的作用域护栏：只认 `/api/v1` 树内，别把 `/api/v10` 这种兄弟路径也算进来。
    #[test]
    fn fallback_scope_is_api_v1_only() {
        assert!(in_api("/api/v1"));
        assert!(in_api("/api/v1/"));
        assert!(in_api("/api/v1/nope"));
        assert!(!in_api("/api/v10"));
        assert!(!in_api("/api/v10/nope"));
        assert!(!in_api("/"));
        assert!(!in_api("/static/app.js"));
    }
}
