# 管理后台服务端 · 核心（M3）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `admin/` 交付管理后台服务端骨架并跑通鉴权链路：INI 配置、8 张表模型、syncdb+种子、JWT 登录/登出、个人资料、动态菜单、登录记录。

**Architecture:** `bee_router`（axum 封装）提供路由；`bee_orm`（M1/M2 交付）提供数据访问；`Auth` 提取器做 JWT 校验 + 权限码加载，handler 内 `auth.require("…")` 逐接口鉴权；数据权限由 `datascope` 模块注入查询条件。

**Tech Stack:** Rust 1.99 / edition 2024、bee_rust（router+logs+config）、bee_orm（path 依赖）、axum 0.8、sqlx（经 bee_orm）、jsonwebtoken 9、argon2 0.5、chrono、serde_json。

**设计依据:** `docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md` §5；接口 JSON 形状以 `docs/superpowers/plans/2026-10-05-admin-web-frontend.md` 的「接口契约」为准（前端已按它开工，不得偏离）。

**前置:** M1/M2（`docs/superpowers/plans/2026-10-05-bee-orm-execution-layer.md`）必须已完成并合入当前分支。开工前先跑 `cargo test -p bee_orm` 确认 ORM 可用。

**测试库:** `bee_admin_test`（已建）。测试启动真实进程，配置见 `admin/conf/app.conf.test`。

**已知约束（重要）:**
- `#[derive(Model)]` 生成的代码引用 `bee_orm::…` 路径 → `admin` 必须**直接**依赖 `bee_orm`（不能只通过 `bee_rust::bee_orm` 用）。
- bee_orm 的 syncdb 不支持复合主键 → 三张连接表（`admin_role`/`role_menu`/`role_dept`）用裸 DDL 幂等创建。
- 菜单表列名用 `menu_type`（避免 Rust 关键字 `type`），JSON 里通过 `#[serde(rename = "type")]` 保持前端契约不变。

---

## 文件结构

```
admin/
  Cargo.toml                 包名 bee_admin；path 依赖 ../crates/bee_rust 与 ../crates/bee_orm
  conf/app.conf.example      提交；app.conf / app.conf.test 真配置（gitignore）
  src/main.rs                启动：配置→连库→syncdb→seed→路由→serve
  src/config.rs              INI 读取（bee_config::ini）+ 校验
  src/state.rs               AppState { db, cfg }
  src/error.rs               ApiError + 统一信封 + From<OrmError>
  src/util.rs                时间格式化、IP 提取、分页参数
  src/auth.rs                JWT 签发/校验 + Auth 提取器 + require()
  src/datascope.rs           数据权限解析与应用
  src/seed.rs                连接表 DDL + 菜单种子 + 超管
  src/models/mod.rs          模型导出
  src/models/{admin,dept,role,menu,login_log,relations}.rs
  src/api/mod.rs             api 模块导出
  src/api/auth.rs            登录/登出/profile/menus/改密
  src/api/{admin,role,menu,dept,login_log}.rs    → M4 计划
  tests/api_core_test.rs     真库集成测试（登录链路）
```

---

### Task B1: crate 骨架 + 配置 + 错误 + 状态

**Files:**
- Create: `admin/Cargo.toml`, `admin/conf/app.conf.example`, `admin/conf/app.conf.test`, `admin/.gitignore`, `admin/src/main.rs`, `admin/src/config.rs`, `admin/src/state.rs`, `admin/src/error.rs`
- Modify: 根 `Cargo.toml`（members 加 `"admin"`）

- [ ] **Step 1: 写 `admin/Cargo.toml`**

```toml
[package]
name = "bee_admin"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"
publish = false

[dependencies]
bee_rust = { version = "1.1.5", path = "../crates/bee_rust" }
# 派生宏生成的代码引用 bee_orm::… 路径，必须直接依赖
bee_orm = { version = "1.1.5", path = "../crates/bee_orm" }
axum = { version = "0.8" }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
jsonwebtoken = "9"
argon2 = { version = "0.5", features = ["std"] }
# password-hash 依赖的 rand_core 默认不带 getrandom，而 OsRng 在 #[cfg(feature = "getrandom")] 后面；
# 必须显式打开这个 feature（cargo 的 feature 合并会作用到 argon2 内部那份 rand_core）
rand_core = { version = "0.6", features = ["getrandom"] }
tracing = "0.1"

[dev-dependencies]
# 工作区已有声明，集成测试发 HTTP 用
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

- [ ] **Step 2: 根 `Cargo.toml` 的 `members` 增加 `"admin"`**

- [ ] **Step 3: 写配置样例与测试配置**

`admin/conf/app.conf.example`：

```ini
[app]
name = bee-rust-admin
http_addr = 127.0.0.1:8080
log_level = info

[db]
dsn = mysql://user:password@127.0.0.1:3306/bee_admin

[jwt]
# 至少 32 字符，且不能是 changeme（否则拒绝启动）
secret = change-me-to-a-random-string-of-32-chars
expire_hours = 24

[seed]
initial_admin_password = admin123
```

`admin/conf/app.conf.test`（提交，测试用；库是 `bee_admin_test`）：

```ini
[app]
name = bee-rust-admin-test
http_addr = 127.0.0.1:0
log_level = warn

[db]
dsn = mysql://root:PASSWORD@127.0.0.1:3306/bee_admin_test

[jwt]
secret = test-secret-0123456789abcdefghijklmnop
expire_hours = 1

[seed]
initial_admin_password = admin123
```

**不要把真实 MySQL 密码提交进 `app.conf.test`。** 该文件用 `include_str!` 之外的方式读取；测试通过环境变量 `BEE_ADMIN_DB_DSN` 覆盖 `[db] dsn` 来注入真实密码（见 B6 集成测试）：

环境变量优先级（`Config::load` 内实现）：`BEE_ADMIN_DB_DSN` > INI 的 `[db] dsn`。因此提交的 `app.conf.test` 里 dsn 可写占位值。

`admin/.gitignore`：

```
conf/app.conf
target
```

- [ ] **Step 4: 写 `admin/src/config.rs`（先写失败测试）**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("读取配置失败 {path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("配置缺少 [{section}] {key}")]
    Missing { section: String, key: String },
    #[error("配置非法: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub app_name: String,
    pub http_addr: String,
    pub log_level: String,
    pub db_dsn: String,
    pub jwt_secret: String,
    pub jwt_expire_hours: i64,
    pub initial_admin_password: String,
}

impl AppConfig {
    /// 从 INI 读取；`BEE_ADMIN_DB_DSN` 环境变量可覆盖 `[db] dsn`（测试注入凭据用）。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io { path: path.display().to_string(), source: e })?;
        let map = bee_rust::bee_config::ini::IniParser::parse(&content);
        Self::from_ini(&map)
    }

    pub fn from_ini(map: &HashMap<String, HashMap<String, String>>) -> Result<Self, ConfigError> {
        let get = |section: &str, key: &str| -> Result<String, ConfigError> {
            map.get(section)
                .and_then(|s| s.get(key))
                .cloned()
                .ok_or_else(|| ConfigError::Missing {
                    section: section.to_string(),
                    key: key.to_string(),
                })
        };

        let db_dsn = std::env::var("BEE_ADMIN_DB_DSN").unwrap_or(get("db", "dsn")?);
        let jwt_secret = get("jwt", "secret")?;
        if jwt_secret.len() < 32 || jwt_secret == "changeme" {
            return Err(ConfigError::Invalid(
                "[jwt] secret 至少 32 字符且不能是 changeme".into(),
            ));
        }

        Ok(Self {
            app_name: get("app", "name")?,
            http_addr: get("app", "http_addr")?,
            log_level: get("app", "log_level").unwrap_or_else(|_| "info".into()),
            db_dsn,
            jwt_secret,
            jwt_expire_hours: get("jwt", "expire_hours")?
                .parse()
                .map_err(|_| ConfigError::Invalid("[jwt] expire_hours 必须是整数".into()))?,
            initial_admin_password: get("seed", "initial_admin_password")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[app]
name = bee-rust-admin
http_addr = 127.0.0.1:8080
log_level = info

[db]
dsn = mysql://u:p@127.0.0.1:3306/db

[jwt]
secret = 0123456789012345678901234567890123
expire_hours = 24

[seed]
initial_admin_password = admin123
"#;

    fn parse() -> HashMap<String, HashMap<String, String>> {
        bee_rust::bee_config::ini::IniParser::parse(SAMPLE)
    }

    #[test]
    fn parses_all_fields() {
        let cfg = AppConfig::from_ini(&parse()).unwrap();
        assert_eq!(cfg.app_name, "bee-rust-admin");
        assert_eq!(cfg.http_addr, "127.0.0.1:8080");
        assert_eq!(cfg.db_dsn, "mysql://u:p@127.0.0.1:3306/db");
        assert_eq!(cfg.jwt_expire_hours, 24);
        assert_eq!(cfg.initial_admin_password, "admin123");
    }

    #[test]
    fn rejects_short_secret() {
        let mut map = parse();
        map.get_mut("jwt").unwrap().insert("secret".into(), "short".into());
        assert!(AppConfig::from_ini(&map).is_err());
    }

    #[test]
    fn reports_missing_key() {
        let mut map = parse();
        map.remove("db");
        let err = AppConfig::from_ini(&map).unwrap_err();
        assert!(format!("{err}").contains("[db] dsn"));
    }
}
```

`ConfigError` 用 `thiserror` → `admin/Cargo.toml` 的 dependencies 再加 `thiserror = "2"`。

**`IniParser` 实际行为**（已读过 `crates/bee_config/src/ini.rs`，无需假设）：按行解析，`[节名]` 起新节，无节的键归入 `"default"`；键与值各自 `trim` 后**按原样**存入（键**不**转小写）；行首 `;` 或 `#` 才是注释（行尾注释不剥离）。所以 `from_ini` 的取键必须与 INI 文件里的大小写完全一致——上面样例全用小写键，配置文件也照此写。

- [ ] **Step 5: 写 `admin/src/error.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bee_orm::OrmError;
use serde_json::{Value, json};

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    Forbidden(String),
    NotFound,
    Internal(String),
}

impl ApiError {
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "登录已失效，请重新登录".to_string()),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "资源不存在".to_string()),
            ApiError::Internal(m) => {
                // 细节只进日志，不外泄给客户端
                tracing::error!("内部错误: {m}");
                (StatusCode::INTERNAL_SERVER_ERROR, "服务器内部错误".to_string())
            }
        };
        let body = json!({ "code": status.as_u16(), "msg": msg, "data": Value::Null });
        (status, Json(body)).into_response()
    }
}

impl From<OrmError> for ApiError {
    fn from(e: OrmError) -> Self {
        match e {
            OrmError::DuplicateKey(_) => ApiError::BadRequest("数据已存在（唯一约束冲突）".into()),
            OrmError::NotFound => ApiError::NotFound,
            other => ApiError::Internal(format!("数据库错误: {other}")),
        }
    }
}

/// 成功信封：`{"code":0,"msg":"ok","data":…}`。
pub fn ok<T: serde::Serialize>(data: T) -> Json<Value> {
    Json(json!({ "code": 0, "msg": "ok", "data": data }))
}
```

注意：唯一键冲突在不同接口有不同友好文案（如「用户名已存在」），handler 里捕获 `OrmError::DuplicateKey` 自行转 `ApiError::BadRequest("用户名已存在")`，不要依赖上面的通用文案。

- [ ] **Step 6: 写 `admin/src/state.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use bee_orm::Db;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub cfg: Arc<AppConfig>,
}
```

- [ ] **Step 7: 写 `admin/src/main.rs`（本任务先起一个只有 /health 的服务；B3 接 seed，B5 接业务路由）**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
mod config;
mod error;
mod state;

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
    let state = AppState { db, cfg: std::sync::Arc::new(cfg) };

    let addr = state.cfg.http_addr.clone(); // 必须在 with_state 前取：它会 move state

    let router = bee_rust::bee_router::Router::new()
        .ns("/api/v1", |ns| ns.get("/health", health))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, router).await?;
    Ok(())
}
```

- [ ] **Step 8: 验证**

Run: `cargo build -p bee_admin`
Expected: 编译通过。

Run: `cargo test -p bee_admin`
Expected: config 的 3 个测试 PASS（本 crate 是 bin-only，没有 lib target，所以是 `cargo test -p bee_admin` 而不是 `--lib`）。

Run（真库冒烟，需要密码）:

```bash
BEE_ADMIN_CONF=admin/conf/app.conf.test BEE_ADMIN_DB_DSN='mysql://root:<密码>@127.0.0.1:3306/bee_admin_test' \
  cargo run -p bee_admin &
sleep 2; kill %1
```
Expected: 进程能连上库并打印 `listening on http://127.0.0.1:0`（端口 0 = 随机端口，本任务只验证「配置能读、库能连、服务能起」；HTTP 冒烟在 B5 的集成测试里做）。

- [ ] **Step 9: Commit**

```bash
git add admin/ Cargo.toml Cargo.lock
git commit -m "feat(admin): 服务端骨架（INI 配置/错误信封/状态/健康检查）"
```

---

### Task B2: 八个模型

**Files:**
- Create: `admin/src/models/mod.rs`, `admin.rs`, `dept.rs`, `role.rs`, `menu.rs`, `login_log.rs`, `relations.rs`
- Modify: `admin/src/main.rs`（`mod models;`）

- [ ] **Step 1: 写 `admin/src/util.rs`（时间序列化助手，模型要用）**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use chrono::NaiveDateTime;
use serde::Serializer;

/// `NaiveDateTime` → `"YYYY-MM-DD HH:MM:SS"`（前端直接展示）。
pub fn ser_dt<S: Serializer>(v: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&v.format("%Y-%m-%d %H:%M:%S").to_string())
}

pub fn ser_opt_dt<S: Serializer>(v: &Option<NaiveDateTime>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(dt) => s.serialize_str(&dt.format("%Y-%m-%d %H:%M:%S").to_string()),
        None => s.serialize_none(),
    }
}

/// 当前时间（本地时区 naive）。
pub fn now() -> NaiveDateTime {
    chrono::Local::now().naive_local()
}
```

- [ ] **Step 2: 写 `admin/src/models/admin.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "admin", pk = "id")]
pub struct Admin {
    #[bee(auto)]
    pub id: u64,
    #[bee(unique)]
    pub username: String,
    #[serde(skip_serializing, default)]
    pub password: String,
    pub nickname: String,
    pub email: String,
    pub phone: String,
    pub sex: i8,
    pub avatar: String,
    #[bee(index)]
    pub dept_id: u64,
    #[bee(index)]
    pub status: i8,
    pub is_super: i8,
    pub token_version: i32,
    #[serde(serialize_with = "crate::util::ser_opt_dt")]
    pub last_login_at: Option<NaiveDateTime>,
    pub last_login_ip: String,
    pub remark: String,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
```

注意：`password` 标了 `skip_serializing`，但 API 里**永远不要把 `Admin` 直接整体返回**作为唯一保护——B5 的 profile/user 响应手工构造 JSON 字段（只挑安全字段）。

- [ ] **Step 3: 写 `dept.rs` / `role.rs` / `menu.rs` / `login_log.rs` / `relations.rs`**

```rust
// dept.rs
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "dept", pk = "id")]
pub struct Dept {
    #[bee(auto)] pub id: u64,
    pub parent_id: u64,
    pub name: String,
    pub sort: i32,
    pub leader: String,
    pub phone: String,
    pub status: i8,
    #[serde(serialize_with = "crate::util::ser_dt")] pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")] pub updated_at: NaiveDateTime,
}
```

```rust
// role.rs
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "role", pk = "id")]
pub struct Role {
    #[bee(auto)] pub id: u64,
    pub name: String,
    #[bee(unique)] pub code: String,
    pub sort: i32,
    /// 1 全部 / 2 本部门及以下 / 3 本部门 / 4 仅本人 / 5 自定义
    pub data_scope: i8,
    pub status: i8,
    pub remark: String,
    #[serde(serialize_with = "crate::util::ser_dt")] pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")] pub updated_at: NaiveDateTime,
}
```

```rust
// menu.rs
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "menu", pk = "id")]
pub struct Menu {
    #[bee(auto)] pub id: u64,
    pub parent_id: u64,
    pub name: String,
    /// M 目录 / C 菜单 / F 按钮（列名用 menu_type，JSON 对外叫 type）
    #[serde(rename = "type")]
    pub menu_type: String,
    pub perm: String,
    pub path: String,
    pub component: String,
    pub icon: String,
    pub sort: i32,
    pub visible: i8,
    pub status: i8,
    #[serde(serialize_with = "crate::util::ser_dt")] pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")] pub updated_at: NaiveDateTime,
}
```

```rust
// login_log.rs
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "login_log", pk = "id")]
pub struct LoginLog {
    #[bee(auto)] pub id: u64,
    pub admin_id: u64,
    pub username: String,
    pub ip: String,
    pub user_agent: String,
    /// 1 成功 / 0 失败
    pub status: i8,
    pub msg: String,
    #[serde(serialize_with = "crate::util::ser_dt")] pub created_at: NaiveDateTime,
}
```

```rust
// relations.rs —— 连接表模型：只用于计数/查询，建表走裸 DDL（syncdb 不支持复合主键）
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "admin_role")]
pub struct AdminRole { pub admin_id: u64, pub role_id: u64 }

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "role_menu")]
pub struct RoleMenu { pub role_id: u64, pub menu_id: u64 }

#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "role_dept")]
pub struct RoleDept { pub role_id: u64, pub dept_id: u64 }
```

`models/mod.rs`：

```rust
pub mod admin;
pub mod dept;
pub mod login_log;
pub mod menu;
pub mod relations;
pub mod role;

pub use admin::Admin;
pub use dept::Dept;
pub use login_log::LoginLog;
pub use menu::Menu;
pub use relations::{AdminRole, RoleDept, RoleMenu};
pub use role::Role;
```

- [ ] **Step 4: `main.rs` 加 `mod models; mod util;`，验证编译**

Run: `cargo build -p bee_admin`
Expected: 通过（宏展开的类型映射与 serde 属性都合法）。若宏报「不支持的字段类型」，检查是否有字段漏在支持列表外（`Option<NaiveDateTime>` / `i8` / `i32` / `i64` 都支持）。

- [ ] **Step 5: Commit**

```bash
git add admin/src
git commit -m "feat(admin): 七个业务模型与三张连接表模型"
```

---

### Task B3: syncdb + 种子数据

**Files:**
- Create: `admin/src/seed.rs`
- Modify: `admin/src/main.rs`（启动时 `syncdb` + `seed`）

- [ ] **Step 1: 写 `admin/src/seed.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::models::{Admin, Menu};
use crate::util::{hash_password, now};
use bee_orm::{Db, Model, SyncdbMode};

/// 建表（syncdb Safe）+ 连接表 DDL，幂等。
pub async fn migrate(db: &Db) -> Result<(), bee_orm::OrmError> {
    db.syncdb(
        &[Admin::META, crate::models::Dept::META, crate::models::Role::META,
          Menu::META, crate::models::LoginLog::META],
        SyncdbMode::Safe,
    )
    .await?;

    // 连接表：syncdb 不支持复合主键，用裸 DDL（幂等）
    for ddl in [
        "CREATE TABLE IF NOT EXISTS admin_role (
           admin_id BIGINT UNSIGNED NOT NULL,
           role_id BIGINT UNSIGNED NOT NULL,
           PRIMARY KEY (admin_id, role_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS role_menu (
           role_id BIGINT UNSIGNED NOT NULL,
           menu_id BIGINT UNSIGNED NOT NULL,
           PRIMARY KEY (role_id, menu_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS role_dept (
           role_id BIGINT UNSIGNED NOT NULL,
           dept_id BIGINT UNSIGNED NOT NULL,
           PRIMARY KEY (role_id, dept_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
    ] {
        db.exec_sql(ddl).await?;
    }
    Ok(())
}

/// 首次启动（admin 表空）时写入：超管 + 菜单权限树。
pub async fn seed(db: &Db, cfg: &AppConfig) -> Result<(), bee_orm::OrmError> {
    if Admin::query().count(db).await? > 0 {
        return Ok(());
    }
    tracing::info!("首次启动，写入种子数据");

    // ── 菜单：目录 → 菜单 → 按钮 ─────────────────────────────
    let mut system = Menu {
        id: 0, parent_id: 0, name: "系统管理".into(), menu_type: "M".into(),
        perm: String::new(), path: "/system".into(), component: String::new(),
        icon: "SettingOutlined".into(), sort: 1, visible: 1, status: 1,
        created_at: now(), updated_at: now(),
    };
    db.insert(&mut system).await?;

    // (名称, 路径, 组件, 图标, 权限码前缀, 按钮后缀列表)
    let menus: [(&str, &str, &str, &str, &str, &[&str]); 5] = [
        ("管理员管理", "/system/admin", "system/admin/index", "UserOutlined", "system:admin",
         &["list", "add", "edit", "remove", "resetPwd"]),
        ("角色管理", "/system/role", "system/role/index", "TeamOutlined", "system:role",
         &["list", "add", "edit", "remove"]),
        ("菜单管理", "/system/menu", "system/menu/index", "MenuOutlined", "system:menu",
         &["list", "add", "edit", "remove"]),
        ("部门管理", "/system/dept", "system/dept/index", "ApartmentOutlined", "system:dept",
         &["list", "add", "edit", "remove"]),
        ("登录记录", "/system/login-log", "system/loginlog/index", "LoginOutlined", "system:loginlog",
         &["list", "remove"]),
    ];

    for (i, (name, path, component, icon, prefix, buttons)) in menus.iter().enumerate() {
        let mut m = Menu {
            id: 0, parent_id: system.id, name: name.to_string(), menu_type: "C".into(),
            perm: format!("{prefix}:list"), path: path.to_string(),
            component: component.to_string(), icon: icon.to_string(),
            sort: (i + 1) as i32, visible: 1, status: 1,
            created_at: now(), updated_at: now(),
        };
        db.insert(&mut m).await?;
        for (j, action) in buttons.iter().enumerate() {
            // 菜单自身的 list 权限已挂在菜单上，不重复建按钮
            if *action == "list" {
                continue;
            }
            let label = match *action {
                "add" => "新增", "edit" => "编辑", "remove" => "删除",
                "resetPwd" => "重置密码", other => other,
            };
            let mut b = Menu {
                id: 0, parent_id: m.id, name: label.to_string(), menu_type: "F".into(),
                perm: format!("{prefix}:{action}"), path: String::new(),
                component: String::new(), icon: String::new(),
                sort: (j + 1) as i32, visible: 0, status: 1,
                created_at: now(), updated_at: now(),
            };
            db.insert(&mut b).await?;
        }
    }

    // ── 超管 ────────────────────────────────────────────────
    let mut admin = Admin {
        id: 0, username: "admin".into(),
        password: hash_password(&cfg.initial_admin_password),
        nickname: "超级管理员".into(), email: String::new(), phone: String::new(),
        sex: 0, avatar: String::new(), dept_id: 0, status: 1, is_super: 1,
        token_version: 0, last_login_at: None, last_login_ip: String::new(),
        remark: "内置超管，请登录后立即修改密码".into(),
        created_at: now(), updated_at: now(),
    };
    db.insert(&mut admin).await?;
    tracing::info!("种子完成：超管 admin / {}", cfg.initial_admin_password);
    Ok(())
}
```

`util.rs` 增加：

```rust
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand_core::OsRng;

pub fn hash_password(plain: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .expect("argon2 哈希失败")
        .to_string()
}

pub fn verify_password(plain: &str, hashed: &str) -> bool {
    match PasswordHash::new(hashed) {
        Ok(h) => Argon2::default().verify_password(plain.as_bytes(), &h).is_ok(),
        Err(_) => false,
    }
}
```

（`rand_core` 已在 Cargo.toml 里显式开了 `getrandom`，所以 `rand_core::OsRng` 可直接用——这一点已对着本机缓存的 `rand_core-0.6.4/src/lib.rs:49` 核过：`#[cfg(feature = "getrandom")] pub use os::OsRng;`。）

- [ ] **Step 2: `main.rs` 接入**

```rust
    let db = bee_orm::Db::connect(&cfg.db_dsn).await?;
    seed::migrate(&db).await?;
    seed::seed(&db, &cfg).await?;
```

（`mod seed;` 一并加上。）

- [ ] **Step 3: 验证（真库）**

```bash
export MYSQL_PWD='<密码>'
mysql -uroot -e "DROP DATABASE IF EXISTS bee_admin_test; CREATE DATABASE bee_admin_test CHARACTER SET utf8mb4"
BEE_ADMIN_CONF=admin/conf/app.conf.test \
BEE_ADMIN_DB_DSN='mysql://root:<密码>@127.0.0.1:3306/bee_admin_test' \
  cargo run -p bee_admin &
sleep 3; kill %1
mysql -uroot -D bee_admin_test -e "SHOW TABLES; SELECT username,is_super FROM admin; SELECT COUNT(*) menus FROM menu;"
```
Expected: 8 张表（admin/dept/role/menu/login_log + 三张连接表）、admin 一行（is_super=1）、menu 约 21 行（1 目录 + 5 菜单 + 15 按钮）。
再跑一次 `cargo run` 确认幂等（不重复插入，日志无「首次启动」）。

- [ ] **Step 4: Commit**

```bash
git add admin/src
git commit -m "feat(admin): syncdb 建表、连接表 DDL 与种子数据（超管 + 菜单权限树）"
```

---

### Task B4: JWT 与 Auth 提取器

**Files:**
- Create: `admin/src/auth.rs`
- Modify: `admin/src/main.rs`（`mod auth;`）

- [ ] **Step 1: 写 `admin/src/auth.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::config::AppConfig;
use crate::error::ApiError;
use crate::models::{Admin, Menu, Role};
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use bee_orm::Model;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// admin.id
    pub sub: u64,
    /// token_version：管理员改密/被禁用后自增，旧 token 立即失效
    pub ver: i32,
    pub iat: i64,
    pub exp: i64,
}

pub fn sign_token(admin_id: u64, ver: i32, cfg: &AppConfig) -> Result<(String, i64), ApiError> {
    let now = chrono::Utc::now().timestamp();
    let exp = now + cfg.jwt_expire_hours * 3600;
    let claims = Claims { sub: admin_id, ver, iat: now, exp };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(cfg.jwt_secret.as_bytes()),
    )
    .map_err(|e| ApiError::internal(format!("签发 token 失败: {e}")))?;
    Ok((token, cfg.jwt_expire_hours * 3600))
}

pub fn verify_token(token: &str, cfg: &AppConfig) -> Result<Claims, ApiError> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(cfg.jwt_secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )
    .map(|d| d.claims)
    .map_err(|_| ApiError::Unauthorized)
}

/// 已认证用户：管理员本体 + 角色 + 权限码。
#[derive(Debug)]
pub struct Auth {
    pub admin: Admin,
    pub roles: Vec<Role>,
    pub perms: HashSet<String>,
    pub is_super: bool,
}

impl Auth {
    /// 逐接口权限校验；超管直接放行。
    pub fn require(&self, code: &str) -> Result<(), ApiError> {
        if self.is_super || self.perms.contains(code) {
            return Ok(());
        }
        Err(ApiError::Forbidden(format!("缺少权限：{code}")))
    }

    pub fn role_ids(&self) -> Vec<u64> {
        self.roles.iter().map(|r| r.id).collect()
    }
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;

        let claims = verify_token(token, &state.cfg)?;

        let admin = Admin::query()
            .filter_eq("id", claims.sub)
            .map_err(ApiError::from)?
            .fetch_one(&state.db)
            .await
            .map_err(ApiError::from)?
            .ok_or(ApiError::Unauthorized)?;

        if admin.status != 1 || admin.token_version != claims.ver {
            return Err(ApiError::Unauthorized);
        }

        let is_super = admin.is_super == 1;
        let role_ids = state
            .db
            .get_relations("admin_role", ("admin_id", admin.id), "role_id")
            .await
            .map_err(ApiError::from)?;

        let roles = if role_ids.is_empty() {
            Vec::new()
        } else {
            Role::query()
                .filter_in("id", role_ids)
                .map_err(ApiError::from)?
                .fetch_all(&state.db)
                .await
                .map_err(ApiError::from)?
        };

        let perms = if is_super {
            HashSet::from(["*:*:*".to_string()])
        } else {
            let mut menu_ids: Vec<u64> = Vec::new();
            for rid in roles.iter().filter(|r| r.status == 1).map(|r| r.id) {
                let ids = state
                    .db
                    .get_relations("role_menu", ("role_id", rid), "menu_id")
                    .await
                    .map_err(ApiError::from)?;
                menu_ids.extend(ids);
            }
            menu_ids.sort_unstable();
            menu_ids.dedup();
            if menu_ids.is_empty() {
                HashSet::new()
            } else {
                Menu::query()
                    .filter_in("id", menu_ids)
                    .map_err(ApiError::from)?
                    .fetch_all(&state.db)
                    .await
                    .map_err(ApiError::from)?
                    .into_iter()
                    .filter(|m| !m.perm.is_empty())
                    .map(|m| m.perm)
                    .collect()
            }
        };

        Ok(Auth { admin, roles, perms, is_super })
    }
}
```

- [ ] **Step 2: 加单元测试（签发/校验往返 + 篡改拒绝，无库）**

`admin/src/auth.rs` 末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AppConfig {
        AppConfig {
            app_name: "t".into(),
            http_addr: "127.0.0.1:0".into(),
            log_level: "warn".into(),
            db_dsn: "mysql://x".into(),
            jwt_secret: "0123456789012345678901234567890123".into(),
            jwt_expire_hours: 24,
            initial_admin_password: "admin123".into(),
        }
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let (token, ttl) = sign_token(7, 3, &cfg()).unwrap();
        assert_eq!(ttl, 86400);
        let claims = verify_token(&token, &cfg()).unwrap();
        assert_eq!(claims.sub, 7);
        assert_eq!(claims.ver, 3);
    }

    #[test]
    fn tampered_token_rejected() {
        let (token, _) = sign_token(7, 0, &cfg()).unwrap();
        let mut other = cfg();
        other.jwt_secret = "9999999999999999999999999999999999".into();
        assert!(verify_token(&token, &other).is_err());
        assert!(verify_token("garbage", &cfg()).is_err());
    }
}
```

- [ ] **Step 3: 验证**

Run: `cargo test -p bee_admin`
Expected: auth 的 2 个测试 + config 的 3 个测试 PASS。

- [ ] **Step 4: Commit**

```bash
git add admin/src
git commit -m "feat(admin): JWT 签发校验与 Auth 提取器（权限码加载 + require 校验）"
```

---

### Task B5: 鉴权接口（登录/登出/资料/菜单/改密）+ 登录记录

**Files:**
- Create: `admin/src/api/mod.rs`, `admin/src/api/auth.rs`
- Modify: `admin/src/main.rs`（挂载路由）
- Create: `admin/tests/api_core_test.rs`

- [ ] **Step 1: 写 `admin/src/api/auth.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::auth::{Auth, sign_token};
use crate::error::{ApiError, ok};
use crate::models::{Admin, LoginLog, Menu, Role};
use crate::state::AppState;
use crate::util::{hash_password, now, verify_password};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use bee_orm::Model;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct ChangePasswordBody {
    pub old_password: String,
    pub new_password: String,
}

/// 客户端 IP：nginx 透传的 X-Real-IP → X-Forwarded-For 首个 → unknown。
pub fn client_ip(headers: &HeaderMap) -> String {
    headers
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
        .unwrap_or_else(|| "unknown".into())
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
    admin_id: u64,
    username: &str,
    headers: &HeaderMap,
    status: i8,
    msg: &str,
) {
    let mut log = LoginLog {
        id: 0,
        admin_id,
        username: username.chars().take(64).collect(),
        ip: client_ip(headers).chars().take(45).collect(),
        user_agent: user_agent(headers),
        status,
        msg: msg.chars().take(255).collect(),
        created_at: now(),
    };
    if let Err(e) = state.db.insert(&mut log).await {
        // 登录记录写失败不能影响登录本身
        tracing::error!("写登录记录失败: {e}");
    }
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Result<Json<Value>, ApiError> {
    if body.username.trim().is_empty() || body.password.is_empty() {
        return Err(ApiError::BadRequest("用户名和密码不能为空".into()));
    }

    let admin = Admin::query()
        .filter_eq("username", body.username.trim())
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?;

    let Some(admin) = admin else {
        write_login_log(&state, 0, &body.username, &headers, 0, "用户不存在").await;
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    };
    if !verify_password(&body.password, &admin.password) {
        write_login_log(&state, admin.id, &admin.username, &headers, 0, "密码错误").await;
        return Err(ApiError::BadRequest("用户名或密码错误".into()));
    }
    if admin.status != 1 {
        write_login_log(&state, admin.id, &admin.username, &headers, 0, "账号已禁用").await;
        return Err(ApiError::BadRequest("账号已被禁用".into()));
    }

    let (token, expires_in) = sign_token(admin.id, admin.token_version, &state.cfg)?;

    let mut updated = admin.clone();
    updated.last_login_at = Some(now());
    updated.last_login_ip = client_ip(&headers);
    updated.updated_at = now();
    state.db.update(&updated).await.map_err(ApiError::from)?;

    write_login_log(&state, admin.id, &admin.username, &headers, 1, "登录成功").await;

    Ok(ok(json!({
        "token": token,
        "expires_in": expires_in,
        "user": {
            "id": admin.id,
            "username": admin.username,
            "nickname": admin.nickname,
            "avatar": admin.avatar,
            "is_super": admin.is_super == 1,
            "dept_id": admin.dept_id,
        }
    })))
}

pub async fn logout(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let mut admin = auth.admin.clone();
    admin.token_version += 1; // 当前 token 立即失效
    admin.updated_at = now();
    state.db.update(&admin).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

pub async fn profile(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    let mut perms: Vec<&String> = auth.perms.iter().collect();
    perms.sort();
    let roles: Vec<&str> = auth.roles.iter().map(|r| r.code.as_str()).collect();
    Ok(ok(json!({
        "user": {
            "id": auth.admin.id,
            "username": auth.admin.username,
            "nickname": auth.admin.nickname,
            "avatar": auth.admin.avatar,
            "is_super": auth.is_super,
            "dept_id": auth.admin.dept_id,
        },
        "roles": roles,
        "perms": perms,
    })))
}

/// 当前用户的菜单树（只含目录/菜单；超管全量，否则按角色勾选）。
pub async fn menus(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let all = Menu::query()
        .filter_eq("status", 1)
        .map_err(ApiError::from)?
        .filter_eq("visible", 1)
        .map_err(ApiError::from)?
        .order_by("sort ASC, id ASC")
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;

    let allowed: Option<Vec<u64>> = if auth.is_super {
        None
    } else {
        let mut ids: Vec<u64> = Vec::new();
        for rid in auth.roles.iter().filter(|r| r.status == 1).map(|r| r.id) {
            ids.extend(
                state
                    .db
                    .get_relations("role_menu", ("role_id", rid), "menu_id")
                    .await
                    .map_err(ApiError::from)?,
            );
        }
        Some(ids)
    };

    let visible: Vec<&Menu> = all
        .iter()
        .filter(|m| m.menu_type == "M" || m.menu_type == "C")
        .filter(|m| match &allowed {
            None => true,
            Some(ids) => ids.contains(&m.id),
        })
        .collect();

    fn build(parent: u64, nodes: &[&Menu]) -> Vec<Value> {
        nodes
            .iter()
            .filter(|m| m.parent_id == parent)
            .map(|m| {
                json!({
                    "id": m.id,
                    "parent_id": m.parent_id,
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

pub async fn change_password(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<ChangePasswordBody>,
) -> Result<Json<Value>, ApiError> {
    if body.new_password.len() < 6 {
        return Err(ApiError::BadRequest("新密码至少 6 位".into()));
    }
    if !verify_password(&body.old_password, &auth.admin.password) {
        return Err(ApiError::BadRequest("原密码错误".into()));
    }
    let mut admin = auth.admin.clone();
    admin.password = hash_password(&body.new_password);
    admin.token_version += 1; // 全端下线，需重新登录
    admin.updated_at = now();
    state.db.update(&admin).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

/// 分页查询参数（各列表接口共用）。
#[derive(Deserialize)]
pub struct PageQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
}

impl PageQuery {
    /// 返回 (1 起始页码, 每页条数)；size 上限 100。
    pub fn page_size(&self) -> (usize, usize) {
        (
            self.page.unwrap_or(1).max(1) as usize,
            self.size.unwrap_or(10).clamp(1, 100) as usize,
        )
    }
}
```

`api/mod.rs`：

```rust
pub mod auth;
```

- [ ] **Step 2: `main.rs` 挂路由**

```rust
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
```

（`mod api;` 一并加上。）

- [ ] **Step 3: 写集成测试 `admin/tests/api_core_test.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 起真实进程 + 真库（bee_admin_test）跑登录链路。需要 BEE_ADMIN_DB_DSN。
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn dsn() -> Option<String> {
    match std::env::var("BEE_ADMIN_DB_DSN") {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("跳过：未设置 BEE_ADMIN_DB_DSN");
            None
        }
    }
}

async fn start_server() -> (Server, String) {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    // 测试配置：改写 app.conf.test 的 http_addr 到随机端口
    let conf = std::fs::read_to_string("conf/app.conf.test").unwrap();
    let conf = conf.replace("127.0.0.1:0", &format!("127.0.0.1:{port}"));
    let tmp = std::env::temp_dir().join(format!("bee_admin_test_{port}.conf"));
    std::fs::write(&tmp, conf).unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_bee_admin"))
        .env("BEE_ADMIN_CONF", &tmp)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("启动 bee_admin 失败");
    let server = Server(child);

    let base = format!("http://127.0.0.1:{port}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if reqwest::get(format!("{base}/api/v1/health")).await.is_ok() {
            return (server, base);
        }
        assert!(tokio::time::Instant::now() < deadline, "服务 15 秒内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn login_profile_menus_logout() {
    let Some(dsn) = dsn() else { return };
    // 每次从干净库开始：删表后由服务启动时的 syncdb/seed 重建
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    for t in ["admin_role", "role_menu", "role_dept", "login_log", "menu", "role", "dept", "admin"] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {t}")).execute(&pool).await.unwrap();
    }
    drop(pool);

    let (_server, base) = start_server().await;
    let client = reqwest::Client::new();

    // 登录失败：密码错
    let r = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&serde_json::json!({"username": "admin", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_ne!(body["code"], 0);

    // 登录成功
    let r = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&serde_json::json!({"username": "admin", "password": "admin123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["code"], 0);
    let token = body["data"]["token"].as_str().unwrap().to_string();
    assert_eq!(body["data"]["user"]["is_super"], true);

    // 未带 token → 401
    let r = client.get(format!("{base}/api/v1/auth/profile")).send().await.unwrap();
    assert_eq!(r.status(), 401);

    // profile
    let r = client
        .get(format!("{base}/api/v1/auth/profile"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["data"]["user"]["username"], "admin");
    assert_eq!(body["data"]["perms"][0], "*:*:*");

    // menus：目录 + 5 个菜单
    let r = client
        .get(format!("{base}/api/v1/auth/menus"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    let root = &body["data"][0];
    assert_eq!(root["path"], "/system");
    assert_eq!(root["children"].as_array().unwrap().len(), 5);

    // 登出后旧 token 失效（token_version +1）
    let r = client
        .post(format!("{base}/api/v1/auth/logout"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = client
        .get(format!("{base}/api/v1/auth/profile"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // 登录记录：1 次失败 + 1 次成功
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM login_log")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
}
```

测试需要 `sqlx` 作为 dev-dependency：`admin/Cargo.toml` 加
`sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "mysql", "tls-none"] }`
（与 bee_orm 用同一版本，不重复编译）。

- [ ] **Step 4: 跑集成测试**

```bash
cd /home/wwwroot/bee-rust-admin
BEE_ADMIN_DB_DSN='mysql://root:<密码>@127.0.0.1:3306/bee_admin_test' \
  cargo test -p bee_admin --test api_core_test -- --nocapture
```
Expected: `login_profile_menus_logout` PASS。

- [ ] **Step 5: Commit**

```bash
git add admin/
git commit -m "feat(admin): 登录/登出/资料/菜单/改密接口与登录记录，含真库集成测试"
```

---

## 后续

M4（业务 CRUD：管理员/角色/菜单/部门/登录记录 + 数据权限）见
`docs/superpowers/plans/2026-10-05-admin-backend-api.md`（在 M3 完成后编写，接口契约已在前端计划中钉死）。

## 已知取舍

- **登录失败锁定/验证码**：不做（设计文档范围外）。
- **权限查询每请求查库**：管理员规模小（<100），不做缓存；需要时加 bee_cache。
- **`is_super` 角色不写 role_menu**：超管走 `is_super` 旁路，角色表里不建超管角色。
- **双写 updated_at**：由 handler 显式设置，不做 DB 触发器/ORM 钩子。
