// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
mod api;
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

async fn health() -> &'static str {
    "OK"
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log = bee_rust::init()?;
    let conf_path = std::env::var("BEE_ADMIN_CONF").unwrap_or_else(|_| "conf/app.conf".into());
    let cfg = AppConfig::load(&conf_path)?;
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
                .get("/menus", api::auth::menus)
                .put("/password", api::auth::change_password)
        })
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, router).await?;
    Ok(())
}
