# bee-rust 管理后台 设计文档

日期：2026-10-05
状态：与用户逐块确认完毕（ORM / 后端 / 前端+部署）。nginx 用端口 8081 对外。

## 1. 目标

在 bee-rust 框架（本仓库 `crates/`）之上交付一个完整的管理后台：登录（JWT）、
管理员管理、角色管理、菜单+按钮权限、部门、登录记录、数据权限，前端 React + antd。

分两个子项目，按序实施：

- **A. bee_orm 执行层**（框架级，先做）：现框架 ORM 只有 SQL 拼接，无连接/执行/迁移能力，
  管理后台无法直接使用。补齐后成为框架能力，后续项目复用。
- **B. 管理后台应用**（`admin/` 服务端 + `admin/web/` 前端）。

## 2. 调研结论（现状事实）

| 组件 | 现状 | 本项目是否使用 |
|---|---|---|
| bee_router | axum 封装：`Router::new().ns(prefix, \|ns\| ns.get(...))` + `with_state()`，`build()` 返回 axum Router（可继续 `.layer()`）。beego 风格 `Controller/Filter/Context::dispatch` 链路**未接线**（全仓库无调用方） | 用，axum handler 风格 |
| bee_orm | 只有 `QuerySet` SQL 拼接（参数化 `filter_eq/gt/lt/contains`），无连接池/执行/CRUD/迁移 | 先补齐（子项目 A） |
| bee_session | 可用（cookie + bee_cache 内存后端） | 不用（选 JWT） |
| bee_config | INI 解析 + `derive(Config)` + reload/watch | 用，读 `conf/app.conf` |
| security-rust 1.0.7 | XSS/SQLi/SSRF 等攻击检测扫描器，**非加密库** | 不用（密码用 argon2，JWT 用 jsonwebtoken） |
| 环境 | Rust 1.99 (edition 2024)、Node 22 + pnpm 10、MySQL 8.4.4 (utf8mb4_0900_ai_ci)、nginx 1.26 | — |
| 数据库 | 新建 `bee_admin`（正式）、`bee_admin_test`（测试）。凭据只进 `admin/conf/app.conf`（gitignore），仓库提交 `.example` | — |

## 3. 决策记录

| 决策点 | 选择 | 理由 |
|---|---|---|
| 数据访问 | 先补齐 bee_orm 再开发后台 | 用户选择；框架能力沉淀 |
| ORM 实现 | 完整 ORM：Active Record + 关联 + syncdb 迁移 | 用户选择 |
| 执行引擎 | sqlx（mysql 先行） | 连接池/预编译/行映射/多库白送，删三个未实现的 optional 依赖 |
| 鉴权 | JWT（HS256）+ `token_version` 强制下线 | 用户选择；不用黑名单表 |
| 权限粒度 | 菜单 + 按钮 + 数据权限 | 用户选择；需部门表 |
| 前端 | Vite + React + TS + Ant Design 5 | 用户选择 |
| 部署 | systemd(127.0.0.1:8080) + nginx(端口 8081) | 用户选择端口方式 |

## 4. 子项目 A：bee_orm 执行层

### 4.1 依赖变更（crates/bee_orm/Cargo.toml）

- 增加 `sqlx`：features `runtime-tokio`、`mysql`；`postgres`、`sqlite` 作为本 crate 的
  feature 预留（sqlx 对应 feature 打开；Postgres 占位符 `?`→`$n` 转换，约 14 行，
  本机无 Postgres，标注 ponytail 注释不测）。
- 删除从未实现的 optional 依赖：`rusqlite`、`tokio-postgres`、`mysql_async`。
- 增加 `chrono`（DATETIME 映射）、`futures`（如需流式）；`serde_json` 已有。

### 4.2 模型宏（bee_orm_macro + 字段属性）

```rust
#[derive(Model)]
#[bee(table = "admin", pk = "id")]
pub struct Admin {
    #[bee(auto)]     pub id: u64,               // 自增主键
    #[bee(unique)]   pub username: String,
    pub password: String,
    #[bee(index)]    pub dept_id: u64,
    pub status: i8,
    pub created_at: chrono::NaiveDateTime,
}
```

宏生成：

- `impl Model for Admin { const META: ModelMeta; }`（table/pk/列名+列类型+auto/unique/index 标记）
- `impl sqlx::FromRow<'_, MySqlRow> for Admin`：按列名 `try_get`，不要求字段顺序
- `Admin::query() -> QuerySet<Admin>`（保留现有签名，表名取自 META）
- insert/update/delete 语句由 META 生成（`db.insert/update/delete` 内部使用）

字段→列名：默认同名字段名（snake_case 即列名），不做驼峰转换魔法。

### 4.3 Db API（bee_orm::Db，唯一对外入口）

```rust
let db = bee_orm::Db::connect("mysql://user:pass@127.0.0.1:3306/bee_admin").await?;

db.insert(&mut admin).await?;          // INSERT，回填 #[bee(auto)] 的 id
db.read::<Admin>(id).await?;           // 主键读，None=不存在
db.update(&admin).await?;              // 按主键 UPDATE 全部列
db.delete::<Admin>(id).await?;         // 按主键 DELETE

// 查询（QuerySet 扩展执行方法，SQL 生成保持参数化）
let rows  = Admin::query().filter_eq("status", 1)?.order_by("id DESC").page(2, 10).fetch_all(&db).await?;
let one   = Admin::query().filter_eq("id", id)?.fetch_one(&db).await?;       // Option<Admin>
let total = Admin::query().filter_eq("status", 1)?.count(&db).await?;
let pages = Admin::query().order_by("id DESC").fetch_page(&db, 2, 10).await?; // (rows, total)

// 新增过滤（数据权限用）
qs.filter_in("dept_id", &[1,2,3])?;                    // 生成 IN (?,?,?)，逐个绑定
                                                       // 空数组 → 恒假（WHERE 1=0），不报错
qs.filter_raw("(status = ? OR is_super = 1)", &["1"])?; // QuerySet 表达不了的复杂条件，
                                                        // 参数仍走绑定，SQL 片段由调用方写死

// M2M（通用连接表方法，覆盖 admin_role / role_menu / role_dept）
db.set_relations("admin_role", ("admin_id", 1), "role_id", &[2, 3]).await?; // 事务: 删旧+批插
db.get_relations("admin_role", ("admin_id", 1), "role_id").await?;          // Vec<u64>
db.del_relations("admin_role", ("admin_id", 1)).await?;                     // 按条件删

// 迁移
db.syncdb(&[Admin::META, Role::META, /* ... */], SyncdbMode::Safe).await?;
let mut tx = db.begin().await?;   // Tx 上同样有 insert/read/update/delete/exec_sql
tx.insert(&mut x).await?;
tx.commit().await?;               // 或 tx.rollback().await?
// 注意：把 &mut Tx 交给异步闭包会撞生命周期/HRTB，begin/commit/rollback 是等价且更简单的形式

// 裸 SQL（DDL / 建测试库 / 运维用；仅限写死的可信 SQL，参数化查询走 QuerySet）
db.exec_sql("CREATE DATABASE IF NOT EXISTS bee_admin_test").await?;
db.exec_sql("SET NAMES utf8mb4").await?;
```

`QuerySet` 现已含 `filter_eq/gt/lt/contains/order_by/limit/offset/to_sql`，本项目新增：
`filter_in`、`filter_raw`、`page(n, size)`、`fetch_all/fetch_one/count/fetch_page`。

### 4.4 行映射与类型

| Rust | MySQL | 说明 |
|---|---|---|
| `u64/i64/u32/i32/i16/i8` | BIGINT / INT / TINYINT | — |
| `String` | VARCHAR(255) | `#[bee(text)]` → TEXT；`#[bee(len = 512)]` → VARCHAR(512) |
| `bool` | TINYINT(1) | — |
| `Option<T>` | 可空列 | — |
| `f64` | DOUBLE | — |
| `chrono::NaiveDateTime` | DATETIME | — |
| `serde_json::Value` | JSON | — |

### 4.5 syncdb（迁移）

- 读 `information_schema` 与模型 META 对比：缺表 `CREATE TABLE`、缺列 `ALTER TABLE ADD COLUMN`。
- `SyncdbMode::Safe`（默认）：只增不删、不改类型、不删列。
- `SyncdbMode::Force`：预留接口，本期不实现（调用返回 `OrmError::Unsupported`）。
- 索引与唯一键由 `#[bee(index)]` / `#[bee(unique)]` 生成（Safe 模式下缺失的索引会补建）。

### 4.6 错误

扩展现有 `OrmError`：增加 `Sqlx(#[from] sqlx::Error)`、`Row(String)`（映射失败）、
`Unsupported(String)`、`DuplicateKey`（MySQL 1062 归一化，供上层转友好提示）。
不 panic；`to_sql()` 的"仅调试"警告保留。

### 4.7 测试

- **单元（无库）**：SQL 生成（filter/page/count/insert/update/delete/syncdb DDL）、`filter_in` 空数组
  恒假、字段名校验拒绝注入。
- **集成（真 MySQL）**：环境变量 `BEE_ORM_TEST_DSN`，未设置则 skip。建临时库 →
  syncdb → 全类型 CRUD → QuerySet 过滤/分页/count → M2M set/get → 删库。
- 本机验证 DSN 用 `bee_admin_test` 库（凭据见 `admin/conf/app.conf`，不入库）。

## 5. 子项目 B：管理后台

### 5.1 表结构（8 张，utf8mb4）

```sql
-- 管理员
admin(
  id BIGINT UNSIGNED AUTO_INCREMENT PK,
  username VARCHAR(64) NOT NULL UNIQUE,
  password VARCHAR(255) NOT NULL,        -- argon2id hash
  nickname VARCHAR(64) NOT NULL DEFAULT '',
  email VARCHAR(128) NOT NULL DEFAULT '',
  phone VARCHAR(20) NOT NULL DEFAULT '',
  sex TINYINT NOT NULL DEFAULT 0,        -- 0未知 1男 2女
  avatar VARCHAR(255) NOT NULL DEFAULT '',
  dept_id BIGINT UNSIGNED NOT NULL DEFAULT 0,
  status TINYINT NOT NULL DEFAULT 1,     -- 1启用 0禁用
  is_super TINYINT NOT NULL DEFAULT 0,
  token_version INT NOT NULL DEFAULT 0,
  last_login_at DATETIME NULL,
  last_login_ip VARCHAR(45) NOT NULL DEFAULT '',
  remark VARCHAR(255) NOT NULL DEFAULT '',
  created_at DATETIME NOT NULL, updated_at DATETIME NOT NULL,
  KEY idx_dept(dept_id), KEY idx_status(status))

-- 部门
dept(id, parent_id BIGINT DEFAULT 0, name VARCHAR(64), sort INT DEFAULT 0,
     leader VARCHAR(64) DEFAULT '', phone VARCHAR(20) DEFAULT '',
     status TINYINT DEFAULT 1, created_at, updated_at)

-- 角色
role(id, name VARCHAR(64), code VARCHAR(64) UNIQUE, sort INT DEFAULT 0,
     data_scope TINYINT DEFAULT 4,   -- 1全部 2本部门及以下 3本部门 4仅本人 5自定义
     status TINYINT DEFAULT 1, remark VARCHAR(255) DEFAULT '', created_at, updated_at)

-- 菜单/权限
menu(id, parent_id BIGINT DEFAULT 0, name VARCHAR(64),
     type CHAR(1),                   -- M目录 C菜单 F按钮
     perm VARCHAR(128) DEFAULT '', path VARCHAR(128) DEFAULT '',
     component VARCHAR(128) DEFAULT '', icon VARCHAR(64) DEFAULT '',
     sort INT DEFAULT 0, visible TINYINT DEFAULT 1, status TINYINT DEFAULT 1,
     created_at, updated_at)

-- 连接表（双列主键）
admin_role(admin_id, role_id)
role_menu(role_id, menu_id)
role_dept(role_id, dept_id)             -- data_scope=5 自定义时用

-- 登录记录
login_log(id, admin_id BIGINT DEFAULT 0, username VARCHAR(64), ip VARCHAR(45),
          user_agent VARCHAR(255), status TINYINT,   -- 0失败 1成功
          msg VARCHAR(255) DEFAULT '', created_at DATETIME,
          KEY idx_username_time(username, created_at))
```

### 5.2 鉴权与权限

- 登录：`username` 查 `admin` → 状态检查 → `argon2` 校验 → 签发 HS256 JWT
  `{sub: admin_id, ver: token_version, iat, exp}`，有效期读配置（默认 24h）。
- `Auth` 提取器（axum FromRequestParts）：`Authorization: Bearer <token>` → 验签 →
  按 `sub` 查库加载管理员 + 角色 + 菜单权限码集合 + 数据权限范围 → 注入 handler。
  token 无效/管理员被禁/`ver` 不匹配 → 401。
- 权限校验：`auth.require("system:admin:add")?`，`is_super=1` 直接通过。
- 强制下线：`token_version` 自增（改密码、禁用、强制退出时）→ 旧 token 全部失效。
- 登录成功/失败均写 `login_log`（IP 取 `X-Real-IP`/`X-Forwarded-For` 首个，UA 截断 255）。

**权限码（seed 数据）**

```
system                   目录
├─ system:admin          菜单 /system/admin    list add edit remove resetPwd
├─ system:role           菜单 /system/role     list add edit remove
├─ system:menu           菜单 /system/menu     list add edit remove
├─ system:dept           菜单 /system/dept     list add edit remove
└─ system:loginlog       菜单 /system/loginlog list remove
```

### 5.3 API（/api/v1，统一信封 `{code, msg, data}`，code 0=成功，非 0 同 HTTP 状态码）

> v1.2 起新增的接口（操作日志与审计、CSV 导出、个人资料、头像）见
> [`plans/2026-10-06-bra-v1.2-improvements.md`](../plans/2026-10-06-bra-v1.2-improvements.md) 的「新接口契约」。

| 模块 | 接口 | 权限码 |
|---|---|---|
| auth | `POST /auth/login` `{username,password}` → `{token, expires_in, user}` | 公开 |
| | `POST /auth/logout`（ver+1，写日志） | 登录即可 |
| | `GET /auth/profile` → 用户+角色+权限码[] | 登录即可 |
| | `GET /auth/menus` → 按权限过滤的菜单树（M/C 两级，按钮不返） | 登录即可 |
| admins | `GET /admins?page&size&username&status&dept_id`（数据权限过滤） | system:admin:list |
| | `GET /admins/{id}` / `POST /admins` / `PUT /admins/{id}` / `DELETE /admins/{id}` | :list/:add/:edit/:remove |
| | `PUT /admins/{id}/status` `{status}` | :edit |
| | `PUT /admins/{id}/password` `{password}`（重置他人，ver+1） | :resetPwd |
| | `PUT /admins/{id}/roles` `{role_ids}` | :edit |
| | `PUT /auth/password` `{old,new}`（个人改密，ver+1） | 登录即可 |
| roles | `GET /roles?page&size&name&status` / `GET /roles/{id}` / `POST` / `PUT /roles/{id}` / `DELETE /roles/{id}` | system:role:list/add/edit/remove |
| | `GET /roles/{id}/menus` → menu_id[] ；`PUT /roles/{id}/menus` `{menu_ids}` | :list / :edit |
| | `PUT /roles/{id}/depts` `{dept_ids}`（data_scope=5 时） | :edit |
| menus | `GET /menus/tree` / `POST /menus` / `PUT /menus/{id}` / `DELETE /menus/{id}` | system:menu:list/add/edit/remove |
| depts | `GET /depts/tree` / `POST /depts` / `PUT /depts/{id}` / `DELETE /depts/{id}` | system:dept:list/add/edit/remove |
| login-logs | `GET /login-logs?page&size&username&status&start&end`（数据权限过滤） | system:loginlog:list |
| | `DELETE /login-logs`（清空，可带筛选） | system:loginlog:remove |

**业务校验（400 友好提示）**：用户名唯一；不能删除/禁用自己；不能删除 `is_super`；
角色被管理员引用时禁止删除；菜单有子节点或被角色引用时禁止删除；部门有子部门或管理员时禁止删除。

### 5.4 数据权限（datascope.rs）

对 `GET /admins`、`GET /login-logs` 注入条件。单角色：

| data_scope | 条件（admins） | 条件（login-logs） |
|---|---|---|
| 1 全部 | 无 | 无 |
| 2 本部门及以下 | `dept_id IN (递归子部门)` | `admin_id IN (该部门集合的管理员)` |
| 3 本部门 | `dept_id = ?` | 同上（本部门） |
| 4 仅本人 | `admin_id = ?` | `admin_id = ?` |
| 5 自定义 | `dept_id IN (role_dept)` | 同上 |

多角色合并：任一角色为"全部"→ 无条件；否则各角色满足的 id 集合取**并集**。
子部门递归在应用层做（dept 表小，一次全量查询 + 内存递归），再 `filter_in`。
`filter_in` 空集合 → `WHERE 1=0`（无害且语义正确）。

**写路径同样受约束（v1.4 起）**：非列表接口不再只看权限码，还要看数据范围与"不得授予超出自己的东西"。
三道闸门在 `datascope.rs`，接在 `api/admin.rs` 的
detail / create / update / remove / set_status / reset_password / set_roles 上：

1. `ensure_admin_in_scope` —— 目标管理员必须在操作者的数据范围内（超管放行），否则 403
2. `ensure_dept_in_scope` —— 新建/修改写入的 `dept_id` 也必须在范围内（否则 scope=3 的人把自己挪到
   B 部门，下一次请求的数据范围就跟着变了，是条真提权路径）
3. `ensure_roles_grantable` —— 非超管只能授 `data_scope ∈ {3 本部门, 4 仅本人}` 的角色，
   且该角色的权限码集合必须是操作者权限码的子集；角色须存在且启用

**闸门顺序**：排在既有的"不能修改超管/自己"判定**之后**——超管 `dept_id=0` 本就不在任何非超管范围内，
先判范围会把既有的 `400 不能修改超级管理员的状态` 契约变成 403，等于把那条护栏测没了。
效果：跨部门普通目标一律 403；超管/自己仍是语义清晰的 400。

角色列表/详情响应带 **`grantable`** 标记（同一条规则求值），供前端禁用授不出去的选项；
写路径仍走 `ensure_roles_grantable` 硬校验，不依赖前端。

### 5.5 错误与响应

```rust
enum ApiError { BadRequest(String), Unauthorized, Forbidden(String), NotFound, Internal(String) }
```

`IntoResponse`：统一 `{code, msg, data: null}`；`Internal` 只进日志，响应固定文案。
DB `DuplicateKey` → 400「用户名已存在」这类具体提示。

### 5.6 代码结构

```
admin/
  Cargo.toml                # path 依赖 ../crates/bee_rust + bee_orm；jsonwebtoken、argon2、chrono、sqlx(测试)
  conf/app.conf.example     # 提交；app.conf 真配置，gitignore
  src/main.rs               # 配置→连库→syncdb→seed→路由→serve
  src/config.rs  src/state.rs  src/error.rs
  src/auth.rs               # JWT 签发/校验 + Auth 提取器 + require()
  src/datascope.rs
  src/models/{admin,dept,role,menu,login_log}.rs
  src/api/{auth,admin,role,menu,dept,login_log}.rs
  src/seed.rs               # 超管 + 菜单权限种子（admin 表空时执行）
admin/web/                  # 前端（见 §6）
```

包名 `bee_admin`（目录 `admin/`）；工作区 `Cargo.toml` 的 members 增加 `"admin"`。

`conf/app.conf`（INI，bee_config 读取）：

```ini
app_name = bee-rust-admin
run_mode = dev
http_addr = 127.0.0.1:8080
db_dsn = mysql://root:***@127.0.0.1:3306/bee_admin
jwt_secret = <启动时若为 changeme 则拒绝启动>
jwt_expire_hours = 24
initial_admin_password = admin123   # 仅首次 seed 超管时使用，README 要求登录后立即改
```

### 5.7 测试

真库集成测试（沿用 `examples/hello/tests` 的套路：随机端口起真实进程 + 直连 HTTP），
`bee_admin_test` 库，跑全链路：登录失败记日志 → 登录成功拿 token → 建部门/角色/菜单 →
建管理员并分配角色 → 无权限接口得 403 → 数据权限过滤结果正确 → 删除 → 禁用后 401。

## 6. 前端（admin/web）

Vite + React 19 + TS + antd 5 + react-router 7 + axios。状态仅 `AuthContext`
（user/perms/menus）+ localStorage 的 token，不引 zustand/redux。

```
src/
  api/          axios 实例（注入 Bearer、401 跳登录、错误 message）+ 各模块请求
  auth/         AuthContext + usePerm() + <Auth code="system:admin:add"> 包裹组件
  layouts/      BasicLayout：侧栏动态菜单 + 面包屑 + 顶栏用户下拉（改密/退出）
  pages/
    login/            登录（回车提交、错误提示）
    dashboard/        首页：欢迎卡片（当前用户、角色、最近一次登录时间/IP）
    system/admin/     管理员：搜索 + 表格 + 新增/编辑弹窗 + 分配角色 + 重置密码 + 状态开关
    system/role/      角色：表格 + 编辑 + 权限树抽屉 + 数据范围下拉（自定义时出部门树）
    system/menu/      菜单：树形表格 + 编辑（类型/权限码/路径/图标）
    system/dept/      部门：树形表格 + 编辑
    system/login-log/ 登录记录：分页 + 时间段筛选 + 清空
    profile/          个人中心（改密码）
```

- 菜单树来自 `/auth/menus`，icon 字符串 → antd 图标映射表；路由懒加载。
- 按钮权限：`<Auth code="...">` 隐藏无权限按钮（后端仍逐接口鉴权，前端只是体验）。
- 路由 `meta.perm` 不符 → 403 页。
- 前端不写自动化测试（手工点验，见里程碑）。

## 7. 部署

- **服务**：systemd `bee-admin.service`，`WorkingDirectory=/home/wwwroot/bee-rust-admin/admin`，
  `ExecStart=.../admin`，重启策略 `on-failure`，监听 `127.0.0.1:8080`。
- **nginx**：新增 server，`listen 8081;`，`root .../admin/web/dist;`
  `try_files $uri /index.html;`，`location /api/ { proxy_pass http://127.0.0.1:8080; }`
  （带 `X-Real-IP` 透传，登录记录要 IP）。
- **开发**：`cargo run -p bee_admin` + `pnpm dev`（vite proxy `/api` → `127.0.0.1:8080`）。

## 8. 里程碑

| # | 内容 | 验收（可跑） |
|---|---|---|
| M1 | ORM 核心：sqlx 引擎、Db、QuerySet 执行、CRUD、宏改造（table/pk/属性/FromRow）、filter_in/filter_raw | `cargo test -p bee_orm`：单测全绿 + 集成测试（真库）全绿 |
| M2 | ORM 关联与迁移：M2M 三方法、syncdb(Safe)、事务 | 集成测试：建库→syncdb→CRUD→M2M 全绿 |
| M3 | 后端骨架：crate、INI 配置、8 模型、syncdb、seed 超管、JWT 登录/登出、登录记录、profile/menus | 集成测试：登录链路全绿；curl 手验 |
| M4 | 业务 API：管理员/角色/菜单/部门/登录记录 + 数据权限 | 集成测试全链路全绿 |
| M5 | 前端：骨架、登录、布局动态菜单、7 页面、按钮权限 | 浏览器手工走一遍全流程 |
| M6 | 部署：systemd + nginx(8081) + README（含初始账号、改密提醒） | `curl http://127.0.0.1:8081` 打开登录页并登录成功 |

M1–M2 框架活，M3 起后台活。每个里程碑独立可验证，不留到最后一起炸。

## 9. 范围外（明确不做，需要再加）

> 本节在 v1.3 做过一次校准：**已完成**的条目已移除或改写（操作日志/审计、登录失败锁定、
> `log_level` 生效、LIKE 转义、禁用菜单即收权、Query/Path 错误信封、暗色主题、图标按需引入）。
> 下面只列**当前仍未做**的事。

- 字典管理、定时任务、通知公告（若依的其余模块）
- sea-orm 式关联查询 DSL（关联用 `set_relations/get_relations` 三方法覆盖）
- 验证码、密码复杂度策略
- 多租户、部门数据权限之外的自定义数据权限
- **前端测试只覆盖纯逻辑**：已引入 vitest（13 个用例，含变异验证），但没有 jsdom/RTL，
  组件渲染与端到端仍靠手工；CI 里跑 `pnpm test` + `pnpm build`
- **数据权限的写路径校验只覆盖已知用例**：v1.4 起 §5.4 的规则已对非列表接口生效（见下），
  但集成用例只覆盖了 `data_scope=3` 与超管两种操作者；`4 仅本人` / `2 本部门及以下` / `5 自定义`
  走同一分支但未实测
- **角色下拉一次最多 100 条**（`size: 200` 被后端 clamp 到 100）
- **`PUT` 一律是全量覆盖语义**：`PUT /admins/{id}` 不带 `role_ids` 等同清空该管理员的角色、
  `PUT /auth/profile` 不带 `email`/`phone` 等同清空——前端表单始终全量提交，直接调 API 的调用方要注意；
  如需「缺省=不改」得改成 `Option<T>` 语义（那是契约变更）
- **`GET /api/v1/avatar/{id}` 不鉴权**（v1.2 起）：`<img src>` 带不了 Authorization 头，所以头像接口是匿名可读的——猜到 admin id 就能取到对应头像（并区分 404 与 200，可用于枚举 id）。头像属低敏信息，接受；若要收紧，改为签名 URL 或前端取 blob
- **登录锁定的实现与边界**（v1.3 起改用 `security-rust` 的 `throttle`）：账号与来源 IP **共用同一阈值**（`[auth] max_fail` / `lock_minutes`）——这是安全取舍：若 IP 桶更宽松，攻击者能在自己的 IP 额度内反复封受害者账号且不被拦。封禁时长严格等于 `lock_minutes`，到点自动解除（"持续猛敲延长封禁"这一性质已消除）。代价与边界：① 共享出口 IP（公司 NAT/VPN）下 5 次失败会连带封掉整段 IP；② throttle 状态在**内存**里，服务重启即清零（可接受：限流是纵深防御，不是认证闸门）；③ 存储不可用时有意 fail-open（库的设计，非认证主闸门）
- **审计中间件的一处静默**：失败响应体读取异常时会回一个空 body（实际不可达；`SELECT username` 失败已在 v1.2 补上 `tracing::error!`）
- **日志保留只验了启动那一次**：`[log] retain_days`（默认 90，0=永久）在启动时清一遍并每 24h 循环，
  但 24h 循环本身没有测试覆盖（循环写法与限流 purge 任务同形）
- **集成测试共用 `bee_admin_test` 库**：单个 `cargo test` 内安全（cargo 串行跑各二进制），但并发跑两个 cargo 进程会互相删表——这是三个集成测试文件共有的既有性质
