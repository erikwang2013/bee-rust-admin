# bee-rust 管理后台 设计文档

日期：2026-10-05
状态：与用户逐块确认完毕（ORM / 后端 / 前端+部署）。nginx 用端口 8081 对外。

> ## ⚠ 实现变更说明（2026-10-07）
>
> 本文第 4 章「ORM 执行层」描述的是**本项目当初自写的那套实现**（`meta`/`query`/`syncdb`/`db`）。
> 框架侧后来长出了完整实现，2026-10-07 已整体迁到上游那份（框架 1.1.5 → 1.2.3），
> 自写实现已删除。**第 4 章请对照实际代码读**，主要出入：
>
> | 本文写的 | 现在的实现 |
> |---|---|
> | `#[bee(table, pk = "id")]` + 字段 `unique`/`index`/`len` | 主键写成字段级 `#[bee(pk)]`；**没有 unique/index**，列长度用 `sql_type = "VARCHAR(n)"` |
> | `syncdb(&[META], SyncdbMode::Safe)` | `migrate::sync::<M>()`（同样只建表/加列）；**唯一键与索引移到 `seed/schema.rs` 的裸 DDL** |
> | `db.insert(&mut x)`（原地回填主键） | `x.create(db)` 返回读回的实例；`x.insert(db)` **不回填主键** |
> | `fetch_one` / `fetch_all` / `fetch_page` | `one` / `all`；分页是项目补的 `PagingExt` |
> | 只有 MySQL（sqlx） | 多后端（mysql_async / tokio-postgres / rusqlite）；**项目只开 mysql 特性** |
> | `OrmError::DuplicateKey` / `Sqlx` / `Row` | 新枚举只有 `ConnectionError`/`QueryError`/`InvalidField`/`NotFound`；冲突靠 MySQL 报错文本识别 |
> | 事务走 `db.pool()` | `Db` trait 层**没有事务**，走 `pool::mysql::Pool::get()` + `CheckedConn` |
> | 模型 id 是 `u64` | **一律 `i64`**（上游 `Value` 不支持 u64） |
> | 连接表各有模型 | 三个 join 模型已删（上游每模型必须有主键），读写走 `src/relations.rs` 的 `RelationsExt` |
>
> 迁移的完整记录见提交 `85cd00d` 与 `docs/superpowers/plans/` 下的相关文档。

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

### 5.1 表结构（22 张，utf8mb4）

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

> 以上是 v1.0 首发的 8 张。此后累计加到 **22 张**（17 张业务表 + 4 张关系表 + 1 张明文备份表）：
> `audit_log`（v1.2 审计）、`dict_type` / `dict_item`（v1.5 字典）、`job` / `job_log`（v1.6 定时任务）、
> `notice` / `notice_read`（v1.6 通知公告）、`sys_config`（系统配置）、`attachment`（附件中心）、
> `admin_login_device`（登录设备）、`export_task`（导出中心）、`admin_backup_code` / `notify_delivery`
> （F4 备用码与通知失败账本）、`admin_email_phone_backup`（存量 email/phone 加密前的明文备份，
> 确认解密正常后可删，见 docs/faq.md 的运维小节）。
> 列的全貌**以代码为准**：模型表由 `admin/src/models/*.rs` 经
> `migrate::sync` 建（`admin/src/seed/schema.rs` 的 `migrate` 里逐个调用），复合主键/复合唯一键/手写索引的表
> （`admin_role` / `role_menu` / `role_dept` / `notice_read` / `dict_item` / `job_log`）
> 与明文备份表在 `admin/src/seed/schema.rs` 里走裸 DDL。

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
├─ system:admin          菜单 /system/admin      list add edit remove resetPwd
├─ system:role           菜单 /system/role       list add edit remove
├─ system:menu           菜单 /system/menu       list add edit remove
├─ system:dept           菜单 /system/dept       list add edit remove
├─ system:loginlog       菜单 /system/login-log  list remove
├─ system:auditlog       菜单 /system/audit-log  list remove
├─ system:dict           菜单 /system/dict       list add edit remove
├─ system:job            菜单 /system/job        list edit
├─ system:notice         菜单 /system/notice     list add edit remove
├─ system:config         菜单 /system/config     list edit
├─ system:notify         菜单 /system/notify     list edit
├─ system:export         菜单 /system/export     list remove
└─ system:attach         菜单 /system/attach     list add edit remove
```

### 5.3 API（/api/v1，统一信封 `{code, msg, err, args, data}`，code 0=成功，非 0 同 HTTP 状态码）

路由在 `admin/src/main.rs` 里以 `.ns()` 命名空间注册，共 **93 条**（91 个业务接口 +
`/api/v1/health` 探活 + `/api/v1/version` 版本）；**不在这里逐条抄录** ——
完整契约（路径、请求体、响应字段、示例）**以 `/apidoc/api.json` 为准**：该文件由 apidoc 在
启动时物化，开启方式见 README「接口文档站（apidoc-rust）」（`[apidoc] enabled = true` +
`password`）。本节只给模块清单与权限码前缀，历史各版新增接口的契约同样以 api.json 为准。

| 模块 | 文件 | 权限码前缀 |
|---|---|---|
| 登录/登出/资料/菜单/改密、验证码 | `admin/src/api/v1/auth/mod.rs` | 公开或登录即可（无权限码） |
| 动态验证码（TOTP）/备用码/登录设备 | `admin/src/api/v1/totp/mod.rs` | 登录即可（无权限码） |
| 头像读取（公开）与分片上传（登录即可） | `admin/src/api/v1/avatar/mod.rs` | 同上 |
| 仪表盘计数与趋势 | `admin/src/api/v1/dashboard/mod.rs` | 登录即可（无权限码） |
| 管理员（含导出） | `admin/src/api/v1/admin/mod.rs` | `system:admin:` list/add/edit/remove/resetPwd |
| 角色（含菜单/部门授权） | `admin/src/api/v1/role/mod.rs` | `system:role:` list/add/edit/remove |
| 菜单 | `admin/src/api/v1/menu/mod.rs` | `system:menu:` list/add/edit/remove |
| 部门 | `admin/src/api/v1/dept/mod.rs` | `system:dept:` list/add/edit/remove |
| 字典类型/字典项（含导出；下拉数据源登录即可） | `admin/src/api/v1/dict/mod.rs` | `system:dict:` list/add/edit/remove |
| 定时任务/任务日志 | `admin/src/api/v1/job/mod.rs` | `system:job:` list/edit |
| 通知公告（未读/标记已读登录即可） | `admin/src/api/v1/notice/mod.rs` | `system:notice:` list/add/edit/remove |
| 登录记录（含导出） | `admin/src/api/v1/login_log/mod.rs` | `system:loginlog:` list/remove |
| 操作日志/审计（含导出） | `admin/src/api/v1/audit_log/mod.rs` | `system:auditlog:` list/remove |
| 系统配置（公开键免登录） | `admin/src/api/v1/config/mod.rs` | `system:config:` list/edit |
| 通知设置（Webhook/SMTP） | `admin/src/api/v1/notify/mod.rs` | `system:notify:` list/edit |
| 导出中心（异步导出） | `admin/src/api/v1/export/mod.rs` | `system:export:` list/remove（发起另要该类型的查看码） |
| 数据导入（管理员/部门/字典项） | `admin/src/api/v1/import/mod.rs` | 复用各模块的 `:add` 码 |
| 附件中心 | `admin/src/api/v1/attach/mod.rs` | `system:attach:` list/add/edit/remove |
| CSV 导出工具（不注册路由） | `admin/src/api/csv.rs` | —（导出接口用各模块自己的 `:list` 码） |

（其中 `GET /api/v1/health` 无鉴权，注册在 `main.rs` 自身。）

**业务校验（400 友好提示）**：用户名唯一；不能删除/禁用自己；不能删除 `is_super`；
角色被管理员引用时禁止删除；菜单有子节点或被角色引用时禁止删除；部门有子部门或管理员时禁止删除。

### 5.4 数据权限（datascope.rs）

对 `GET /admins`、`GET /login-logs`、`GET /audit-logs`、`GET /attachments` 注入条件
（后两者按记录所属 / 上传人的 `admin_id` 过滤）。单角色：

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
三道闸门在 `datascope.rs`，接在 `api/v1/admin/mod.rs` 的
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

**角色的写路径与菜单的 `perm` 写入同样受 B5 约束（同一口径，后补）**：`role::update` / `role::set_depts` /
`role::set_menus` 用 `ensure_role_writable`（复用 `block_reason`：只能动启用、`data_scope ∈ {3,4}`、权限码
是自己子集的角色；新 `data_scope` 也只允许 3/4 —— 否则 `system:role:edit` 就能把自己的角色改成"全部数据"，
下一个请求即全站可见），`set_menus` 另用 `ensure_menus_attachable`（菜单必须真实存在，权限码得是自己子集），
`menu::create/update` 用 `ensure_perm_writable`（非空的 `perm` 必须是自己已有的码，堵住"就地造码再挂给自己"）。
错误沿用 `block_reason` 的既有映射与稳定码，超管一律放行。

### 5.5 错误与响应

```rust
enum ApiError {
    BadRequest(String), Unauthorized, Forbidden(String), NotFound,
    NotFoundMsg(String), Internal(String),
    Coded { status: StatusCode, err: &'static str, msg: String, args: Value },
}
```

`IntoResponse`：统一 `{code, msg, err, args, data}`；`Internal` 只进日志，响应固定文案；
`Coded` 带稳定错误码与参数（前端查表拼文案）。DB `DuplicateKey` → 400「用户名已存在」这类具体提示。

### 5.6 代码结构

```
admin/
  Cargo.toml                # path 依赖 ../crates/bee_rust + bee_orm；jsonwebtoken、argon2、chrono、sqlx(测试)
  conf/app.conf.example     # 提交；app.conf 真配置，gitignore
  src/main.rs               # 配置→连库→migrate→seed→路由→serve
  src/config.rs  src/state.rs  src/error.rs
  src/auth.rs               # JWT 签发/校验 + Auth 提取器 + require()
  src/datascope.rs
  src/models/*.rs           # 15 个模型（见 §5.1）
  src/api/v1/<模块>/mod.rs  # 18 个业务模块；api/v1/mod.rs 为 v1 入口，api/csv.rs 为内部工具
  src/seed/                 # mod.rs / schema.rs（建表与裸 DDL）/ defaults.rs / menus.rs
  src/jobs/mod.rs           # 8 个内置任务注册与调度
  src/export/               # 异步导出执行 / 清理（export_run / export_clean）
admin/web/                  # 前端（见 §6）
```

> 当前 `src/` 下还有 `audit.rs`（审计中间件）、`crypto` / `hid`（密文列、隐藏 id）、`relations`、
> `retention.rs`、`backup.rs`、`metrics.rs`、`security_scan.rs`、`totp.rs`、`notify/` 等；
> 完整清单以 `admin/src/` 为准。

包名 `bee_admin`（目录 `admin/`）；工作区 `Cargo.toml` 的 members 增加 `"admin"`。

`conf/app.conf`（INI，bee_config 读取）：

```ini
[app]
name = bee-rust-admin
run_mode = dev
http_addr = 127.0.0.1:8080

[db]
dsn = mysql://root:***@127.0.0.1:3306/bee_admin

[jwt]
secret = <启动时若为 changeme 则拒绝启动>
expire_hours = 24

[seed]
initial_admin_password = change-me-please-1   # 仅首次 seed 超管时使用，README 要求登录后立即改
```

### 5.7 测试

真库集成测试（沿用 `examples/hello/tests` 的套路：随机端口起真实进程 + 直连 HTTP），
`bee_admin_test` 库，跑全链路：登录失败记日志 → 登录成功拿 token → 建部门/角色/菜单 →
建管理员并分配角色 → 无权限接口得 403 → 数据权限过滤结果正确 → 删除 → 禁用后 401。

## 6. 前端（admin/web）

Vite + React 18 + TS + antd 5 + react-router 7 + axios。状态仅 `AuthContext`
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

> 以上是 v1.0 的页面。此后加了 `system/dict`（字典）、`system/job`（定时任务 + 执行记录）、
> `system/notice`（通知公告）、`system/auditlog`（操作日志）与 `error/`（403、404）；
> 完整清单以 `admin/web/src/pages/` 为准。

- 菜单树来自 `/auth/menus`，icon 字符串 → antd 图标映射表；路由懒加载。
- 按钮权限：`<Auth code="...">` 隐藏无权限按钮（后端仍逐接口鉴权，前端只是体验）。
- 路由 `meta.perm` 不符 → 403 页。
- 前端测试只覆盖纯逻辑（vitest；组件渲染与端到端仍手工点验，见 §9 与里程碑）。

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
> v2.7 又校准一次：字典/定时任务/通知公告（v1.5/v1.6）与验证码（v2.4）已完成、移出本节，
> 日志保留按任务化后的实现重写。下面只列**当前仍未做**的事。

- sea-orm 式关联查询 DSL（关联用 `set_relations/get_relations` 三方法覆盖）
- 密码复杂度**更严的策略**（熵值、弱密码黑名单、定期更换等）：最小加固（≥8 位 + 至少两类字符，
  只作用于新设/新改密码，登录不校验）已随 v2.8 落地，更严的仍不做。例外：`[seed] initial_admin_password`
  由配置直写哈希、不走该校验（只在库为空时的种子阶段生效）
- 多租户、部门数据权限之外的自定义数据权限
- **前端测试只覆盖纯逻辑**：已引入 vitest（含变异验证；用例数以 `pnpm test` 输出为准），但没有 jsdom/RTL，
  组件渲染与端到端仍靠手工；CI 里跑 `pnpm test` + `pnpm build`
- **角色下拉一次最多 100 条**（后端单页上限，设计如此）：前端请求已对齐 100，`total` 超出时下拉内追加禁用项
  提示「仅显示前 100 条，请在上方输入搜索」；搜索框按角色名过滤（`optionFilterProp="label"`）。超过 100 个角色仍要靠搜索，不翻页
- **`PUT` 一律是全量覆盖语义**：`PUT /admins/{id}` 不带 `role_ids` 等同清空该管理员的角色、
  `PUT /auth/profile` 不带 `email`/`phone` 等同清空——前端表单始终全量提交，直接调 API 的调用方要注意；
  如需「缺省=不改」得改成 `Option<T>` 语义（那是契约变更）
- **`GET /api/v1/avatar/{id}` 不鉴权**（v1.2 起）：`<img src>` 带不了 Authorization 头，所以头像接口是匿名可读的——猜到 admin id 就能取到对应头像（并区分 404 与 200，可用于枚举 id）。头像属低敏信息，接受；若要收紧，改为签名 URL 或前端取 blob
- **登录锁定的实现与边界**（v1.3 起改用 `security-rust` 的 `throttle`）：账号与来源 IP **共用同一阈值**（`[auth] max_fail` / `lock_minutes`）——这是安全取舍：若 IP 桶更宽松，攻击者能在自己的 IP 额度内反复封受害者账号且不被拦。封禁时长严格等于 `lock_minutes`，到点自动解除（"持续猛敲延长封禁"这一性质已消除）。代价与边界：① 共享出口 IP（公司 NAT/VPN）下 5 次失败会连带封掉整段 IP；② throttle 状态在**内存**里，服务重启即清零（可接受：限流是纵深防御，不是认证闸门）；③ 存储不可用时有意 fail-open（库的设计，非认证主闸门）
- **审计中间件的一处静默**：失败响应体读取异常时会回一个空 body（实际不可达；`SELECT username` 失败已在 v1.2 补上 `tracing::error!`）
- **日志保留的覆盖边界**：`[log] retain_days`（默认 90，0=永久）清理 `login_log` / `audit_log` /
  `job_log`（v1.6 起 `job_log` 并入）；启动时先清一遍（删除效果由 `admin/tests/api_v14_test.rs` 覆盖），
  此后由内置任务 `log_retention`（`admin/src/jobs/mod.rs`，默认间隔 86400 秒 = 每天）调用
  `retention::purge_all`。任务路径：调度循环真跑起它、写 `job_log`、回写 `last_run_at` /
  `last_status` 均有覆盖（`admin/tests/api_v16_test.rs` 把间隔改成 2 秒等它自己跑），
  但「任务跑一次实际删掉多少行」没有直接断言
- **集成测试共用 `bee_admin_test` 库**：单个 `cargo test` 内安全（cargo 串行跑各二进制），但并发跑两个 cargo 进程会互相删表——这是 38 个集成测试二进制共有的既有性质
