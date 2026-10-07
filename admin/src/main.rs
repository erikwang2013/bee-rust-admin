// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
mod api;
mod audit;
mod auth;
mod config;
mod crypto;
mod datascope;
mod error;
mod hid;
mod jobs;
mod models;
mod relations;
mod retention;
mod seed;
mod state;
mod util;

use axum::response::IntoResponse;
use config::AppConfig;
use snowflake::{Shared, Snowflake};
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
        error::envelope(axum::http::StatusCode::NOT_FOUND, "接口不存在", Some("common.not_found"))
    } else {
        axum::http::StatusCode::NOT_FOUND.into_response()
    }
}

/// 路由级 405（A1）：路径存在但方法不对，同样回信封。
async fn api_method_not_allowed(uri: axum::http::Uri) -> axum::response::Response {
    if in_api(uri.path()) {
        error::envelope(axum::http::StatusCode::METHOD_NOT_ALLOWED, "方法不允许", None)
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

/// 雪花发号器：节点号/数据中心号来自 `[app] snowflake_worker` / `snowflake_dc`。
/// 多实例部署时各进程必须取不同的号，否则会发出重复 id（唯一键冲突在写入时才暴露）。
fn build_snowflake(cfg: &AppConfig) -> Result<Shared, Box<dyn std::error::Error>> {
    let sk = Snowflake::builder()
        .worker_id(cfg.snowflake_worker)
        .datacenter_id(cfg.snowflake_dc)
        .build()
        .map_err(|e| format!("[app] snowflake_worker / snowflake_dc 非法: {e}"))?;
    Ok(Shared::new(sk))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conf_path = std::env::var("BEE_ADMIN_CONF").unwrap_or_else(|_| "conf/app.conf".into());
    let cfg = AppConfig::load(&conf_path)?;

    // 先读配置再起日志，log_level 才真正生效（B1）；句柄要活到进程结束
    let _log = bee_rust::bee_logs::Logger::new().level(parse_level(&cfg.log_level)).init()?;
    tracing::info!("{} 启动，配置 {}", cfg.app_name, conf_path);

    // 对外 id 的编码参数在启动时定死（进程级 OnceLock，之后所有 serde 辅助都用它）。
    // 盐是空串也能跑（老配置没这个键），但那样短串在不同部署间可预测 —— 提醒一句。
    hid::init(&cfg.hashids_salt, cfg.hashids_min_len)
        .map_err(|e| format!("初始化 hashids 失败: {e}"))?;
    if cfg.hashids_salt.is_empty() {
        tracing::warn!("[app] hashids_salt 未配置，对外 id 短串在所有部署间可预测（生产请设随机串）");
    }
    let snowflake = build_snowflake(&cfg)?;

    // JWT 内核（密钥/算法/issuer 在这里定死，签发与校验共用这一份）
    let jwt = std::sync::Arc::new(auth::build_jwt(&cfg).map_err(|e| format!("初始化 JWT 失败: {e}"))?);

    // email / phone 的落库加密守卫。密钥只在启动时解析一次：配置缺键 / 密钥非法都在
    // 这里炸 —— 绝不让进程带着「以为加密了其实没加密」跑起来（配置校验见 `config.rs`）。
    let crypto = crypto::build_guard(&cfg.encrypt_key)?;

    // 池大小沿用迁移前的 10；`Pool::connect` 是同步的（连接按需惰性建立）
    let db = bee_orm::pool::mysql::Pool::connect(&cfg.db_dsn, 10)?;
    seed::migrate(&db).await?;
    // 存量明文加密（幂等、可重跑）：建表之后、seed 之前 —— 数据正确性不依赖它
    // （读路径兼容明文），越早跑完越少明文滞留。
    seed::encrypt_legacy(&db, &crypto).await?;
    seed::seed(&db, &cfg, &snowflake, &crypto).await?;
    let throttle = api::auth::login_throttle(&cfg);
    let state = AppState { db, cfg: std::sync::Arc::new(cfg), throttle, snowflake, jwt, crypto };
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

    // 日志保留（A4）：启动先清一遍积压——这是**启动卫生**，不是定时任务，
    // 所以不受 `[job] enabled` 影响。每 24 小时的循环改由 `log_retention` 任务承担
    // （v1.6 C2a：同一个实现在「操作日志」之外还能被手动触发）。
    if state.cfg.retain_days > 0 {
        let db = state.db.clone();
        let retain_days = state.cfg.retain_days;
        tokio::spawn(async move {
            retention::purge_all(&db, retain_days).await;
        });
    }

    // 定时任务调度循环（C2a）：`[job] enabled=false` 时干脆不启动；手动触发不依赖它。
    if state.cfg.job_enabled {
        tokio::spawn(jobs::scheduler(state.clone()));
    } else {
        tracing::info!("[job] enabled=false，定时任务调度循环未启动（手动触发仍可用）");
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
        .ns("/api/v1/dicts", |ns| {
            ns.get("", api::dict::type_list)
                .post("", api::dict::type_create)
                .put("/{id}", api::dict::type_update)
                .delete("/{id}", api::dict::type_remove)
                // 下拉数据源：登录即可，注册在 {id} 之后也不冲突（路径形状不同）
                .get("/{code}/items", api::dict::type_items)
        })
        .ns("/api/v1/dict-items", |ns| {
            ns.get("", api::dict::item_list)
                .post("", api::dict::item_create)
                .get("/export", api::dict::item_export)
                .put("/{id}", api::dict::item_update)
                .delete("/{id}", api::dict::item_remove)
        })
        .ns("/api/v1/jobs", |ns| {
            ns.get("", api::job::list)
                .put("/{id}", api::job::update)
                .post("/{id}/run", api::job::run)
        })
        .ns("/api/v1/job-logs", |ns| ns.get("", api::job::log_list))
        .ns("/api/v1/notices", |ns| {
            ns.get("", api::notice::list)
                .post("", api::notice::create)
                // 未读/标记已读：登录即可（不挂权限码），静态段优先于 /{id}
                .get("/unread", api::notice::unread)
                .post("/{id}/read", api::notice::read)
                .put("/{id}", api::notice::update)
                .delete("/{id}", api::notice::remove)
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
