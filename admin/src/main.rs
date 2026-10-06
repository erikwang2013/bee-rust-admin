// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
mod api;
mod audit;
mod auth;
mod config;
mod datascope;
mod error;
mod models;
mod seed;
mod state;
mod util;

use config::AppConfig;
use state::AppState;
use tracing::Level;

async fn health() -> &'static str {
    "OK"
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
    let state = AppState { db, cfg: std::sync::Arc::new(cfg) };
    let addr = state.cfg.http_addr.clone();

    let router = bee_rust::bee_router::Router::new()
        .ns("/api/v1", |ns| ns.get("/health", health))
        .ns("/api/v1/auth", |ns| {
            ns.post("/login", api::auth::login)
                .post("/logout", api::auth::logout)
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
}
