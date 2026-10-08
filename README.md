# BRA · Bee Rust Admin

基于 bee-rust 框架的 RBAC 管理后台 —— Rust 服务端 + React 前端，建库即用。

`JWT 登录` · `管理员 / 角色 / 菜单 / 部门 / 字典 / 定时任务 / 通知公告 / 登录记录` · `菜单 + 按钮级权限` · `部门数据权限` · `中英双语` · `Docker 一键部署`

---

## 项目宠物

<img src="docs/pet/keeper-onduty.svg" width="240" align="right" alt="阿守（Keeper）· BRA 项目宠物：站在六边形巢门平台上、爪挂三把钥匙的值守蜂">

**阿守（Keeper）** —— 蜂巢的值守蜂。

框架宠物 [Rusty](docs/rusty.svg) 是造蜂巢的工蜂；阿守是站在巢门口的那位：**查凭证、发钥匙、登记访客**。
它是这个后台的性格投射——权限不是拦路的墙，是有人认真守着门。

| 形象细节 | 对应功能 / 设定 |
|---|---|
| 胸前**六边形徽章**（对勾 / 金星 / 感叹号） | 蜂巢格 + 查验结果：普通 / 超管 / 告警 |
| 手里提着**一把钥匙** | 权限的授予与收回（对应 `admin_role` / `role_menu` / `role_dept` 三张连接表） |
| **翅膀**平时收在背后 | JWT 平时沉默；超管（旁路权限码）时才展开 |
| **尾针**平时收成小圆点 | 只在「仓库里检出密钥文件」时竖起 |
| 青蓝配色只出现在翅膀与徽章 | 与管理后台界面同色系的点缀，不喧宾夺主 |

**五种状态**（与框架宠物同一套判定机制，由项目状态自动切换；形象由 `docs/pet/render.mjs` 统一渲染）：

| 值守 | 查证 | 超管 | 打盹 | 竖针 |
|:--:|:--:|:--:|:--:|:--:|
| <img src="docs/pet/keeper-onduty.svg" width="112" alt="值守"> | <img src="docs/pet/keeper-verifying.svg" width="112" alt="查证"> | <img src="docs/pet/keeper-superuser.svg" width="112" alt="超管"> | <img src="docs/pet/keeper-dozing.svg" width="112" alt="打盹"> | <img src="docs/pet/keeper-alarmed.svg" width="112" alt="竖针"> |
| `(⌐■_■)` | `(・_・)?` | `(＾▽＾)` | `(-_-)zZ` | `(#°益°)` |
| 门岗正常，权限表与菜单树一致 | 有变更待验证，凭证拿来 | 超管入场，旁路所有权限码 | 这个点没人来访，登录记录空着 | 尾针竖起来了——有人把密钥写进仓库了！ |
| 测试全绿 · 无未提交改动 | 有未提交改动待审 | 在 main · 刚打 tag | 深夜 · 无人来访 | 仓库里检出密钥文件 |

## 项目介绍

**BRA（Bee Rust Admin）** 是一个开箱即用的后台管理系统：服务端用本仓库的
[bee-rust](https://github.com/erikwang2013/bee-rust) 框架写就（axum 路由 + 自研 ORM 执行层 +
INI 配置），前端是 Vite + React 18 + Ant Design 5 单页应用，两者通过一套统一的
`{code, msg, data}` 接口契约对接（错误响应另带稳定的业务错误码 `err` 与参数 `args`，前端据此出多语言文案）。

它解决的问题很实际：**权限要细到按钮、数据要看得到范围、登录要留痕**。因此：

- **权限模型是菜单 + 按钮两级**：角色勾选到按钮级权限码（如 `system:admin:resetPwd`），
  前端据此渲染菜单与按钮，后端**每个接口再校验一次**——前端隐藏只是体验，不是防线。
- **数据权限有五档**：全部 / 本部门及以下 / 本部门 / 仅本人 / 自定义（勾选部门），
  在列表查询层做条件注入，参数化绑定，不拼接用户输入。
- **会话无状态但有强制下线**：JWT 携带 `token_version`，改密 / 禁用 / 登出即失效旧令牌。
- **登录成功 / 失败 / 退出全部留痕**：IP、User-Agent、失败原因入库可查。

服务端启动时自动完成建表与种子（超管 + 菜单权限树），**不需要手工执行 SQL 迁移**。

## 架构设计

![架构设计](docs/diagrams/architecture.svg)

五层结构，自上而下：

| 层 | 组成 | 说明 |
|---|---|---|
| 客户端 | 浏览器 SPA | React 18 + TypeScript + Ant Design 5（Vite 构建） |
| 接入层 | nginx :8081 | 提供前端静态资源，`/api/` 反向代理到后端并透传 `X-Real-IP` |
| 应用层 | `bee_admin` | bee_router（axum）· Auth 提取器 · 62 个 API handler · datascope 数据权限 · 统一响应信封 |
| 框架层 | bee_orm / bee_config / bee_logs | 多后端连接池、QuerySet、CRUD、M2M、migrate 迁移；INI 配置；tracing 日志 |
| 数据层 | MySQL 8.4 | 15 张表（11 张业务表 + 4 张关系表） |

## 功能设计

![功能设计](docs/diagrams/features.svg)

**权限模型是多对多的三段式**：管理员 ←`admin_role`→ 角色 ←`role_menu`→ 菜单与按钮；
角色 ←`role_dept`→ 部门（用于「自定义」数据范围）。超管（`is_super`）旁路所有权限码校验。

**数据权限五档**在列表接口注入查询条件：多角色的部门集合取并集，任一角色为「全部」则不加条件，
一个启用的角色都没有则落到 `WHERE 1 = 0`（什么都不给看，而不是全给看）。

## 请求周期

![请求周期](docs/diagrams/request-cycle.svg)

以 `GET /api/v1/admins` 为例的 12 步链路：浏览器 → nginx → bee_router → Auth 提取器
（校验签名 / `ver` / 账号状态，加载角色与权限码）→ handler（先 `require(权限码)`，再解析数据权限）
→ bee_orm（预编译 + 参数绑定）→ MySQL → 统一信封返回。**401（未登录）与 403（无权限）在查库之前就短路**。

## 服务生命周期

![服务生命周期](docs/diagrams/lifecycle.svg)

启动路径：读配置（JWT secret 少于 32 字符或为 `changeme` → **拒绝启动**）→ 连接 MySQL →
`migrate::sync` 同步表结构（**只建表 / 加列，绝不删列改类型**，模型加字段下次启动自动补列；
唯一键与索引由 `seed.rs` 的裸 DDL 建 —— 上游的模型属性只表达列、主键与外键，没有 unique/index）→
首次启动（`admin` 表为空）在**单事务内**写入超管与菜单权限树 → 监听 `127.0.0.1:8080` → 运行 →
收到 `SIGTERM` 退出（systemd 策略 `on-failure` 自动重启）。

## 功能介绍

| 模块 | 能力 |
|---|---|
| **登录鉴权** | 账号密码登录（argon2id）、JWT 签发与校验、登出、个人中心改密（改后强制重新登录） |
| **管理员管理** | 分页列表（用户名 / 状态 / 部门筛选）、增删改、分配角色、重置密码、启用禁用（禁用即踢下线） |
| **角色管理** | 增删改查、菜单权限树勾选、数据范围五档、自定义部门、删除前校验是否被管理员引用 |
| **菜单管理** | 树形目录 / 菜单 / 按钮三层、权限码、图标与排序、防环校验、删除前校验子节点与角色引用 |
| **部门管理** | 树形部门、负责人与排序、删除前校验子部门与在编管理员 |
| **登录记录** | 按用户名 / 状态 / 时间段分页查询、按条件清空，记录受数据权限约束 |
| **字典管理** | 字典类型 + 字典项两级维护、同类型内 value 唯一、删除类型级联删项、CSV 导出；`GET /dicts/{code}/items` 是登录即可用的下拉数据源（只回启用项） |
| **定时任务** | 内置任务由**代码注册**（code 是注册键，库里只放调度与执行记录，界面不能新增/改名）、固定间隔（秒）调度 + 启停、手动触发一次、执行记录分页；`[job] enabled` 总开关关掉后仍可手动补跑 |
| **通知公告** | 草稿 / 已发布两态（发布写 `published_at`，回退草稿不清空——留痕）、增删改、删公告同事务级联删已读记录；顶栏铃铛显示未读数 |
| **中英双语** | 界面文案中/英两套（顶栏切换，跟随浏览器语言，存 localStorage），antd 组件文案与日期格式跟着切；**后端不做多语言消息**——错误响应带稳定业务错误码 `err` + 参数 `args`，前端查表出文案，查不到回落后端中文 `msg`（任何情况都有话可说） |

接口共 62 个（61 个业务接口 + `/api/v1/health` 探活），路径与字段的完整定义见
[设计文档 §5.3](docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md)，
字典 / 定时任务 / 通知公告 / i18n 的契约另见 [C 组设计](docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md)。

## 项目结构

```
crates/                 bee-rust 框架（workspace 成员，本后台直接依赖）
  bee_orm/              ★ ORM 执行层：多后端连接池 / QuerySet / CRUD / M2M / migrate
  bee_orm_macro/        ★ #[derive(Model)]：元数据 + 行映射 + 参数绑定代码生成
  bee_router/           axum 封装的路由
  bee_config/           INI 配置（含解析与监听）
  bee_logs/             tracing 日志初始化
  bee_cli/ bee_kv/ ...  其余框架 crate
admin/                  管理后台服务端（crate bee_admin）
  src/main.rs           启动：配置 → 连库 → migrate → 种子 → 路由 → 监听
  src/config.rs         INI 配置与启动校验（JWT secret 强度、DSN 覆盖）
  src/error.rs          ApiError + 统一信封（code/msg/err/args）+ 错误码表 + 请求体提取器
  src/auth.rs           JWT 签发校验 + Auth 提取器（权限码加载、token_version 校验）
  src/datascope.rs      数据权限解析与查询注入（部门子树 / 本人 / 自定义）
  src/api/              auth · admin · role · menu · dept · dict · job · notice · login_log · audit_log 十个模块
  src/models/           11 个模型（migrate 建表）。列长度用 `sql_type = "VARCHAR(n)"` 声明；
                        **唯一键与索引不走模型**（上游属性没有这两个概念），集中在
                        `src/seed.rs` 的 CONSTRAINTS 里建。连接表（admin_role/role_menu/
                        role_dept/notice_read）是复合主键、无模型结构体，走 seed 的裸 DDL，
                        读写见 src/relations.rs
  src/seed.rs           建表、连接表 DDL、首次种子（超管 + 菜单权限树）
  conf/                 app.conf（gitignore）· app.conf.example · app.conf.test
  deploy/               systemd 单元 + nginx 站点配置
  tests/                集成测试：起真实进程 + 连真库（含全链路）
  web/                  前端 SPA（React + antd，见下）
    src/api/            接口层（契约类型 + axios 拦截器 + 错误码→文案映射）
    src/i18n/           类型化词表（中英）+ I18nProvider + 语言开关
    src/auth/           AuthContext + 按钮级权限组件
    src/layouts/        动态菜单布局
    src/pages/          登录 / 首页 / 管理员 / 角色 / 菜单 / 部门 / 字典 / 定时任务 / 通知公告 /
                        登录记录 / 个人中心
docs/
  diagrams/             ★ 本 README 引用的 4 张 SVG 图
  pet/                  项目宠物「阿守」的五态 SVG + 渲染器（node render.mjs）
  social/               社交预览图 1280×640（仓库 Settings → Social preview 上传那份 PNG）
  superpowers/specs/    设计文档（数据模型、接口清单、数据权限规则）
  superpowers/plans/    实施计划（ORM 执行层、后端、前端）
Dockerfile              后端镜像（多阶段构建）
docker-compose.yml      MySQL + 后端 + 前端一键起栈
```

## 使用说明

### 环境要求

| 组件 | 版本 |
|---|---|
| Rust | 1.99+（edition 2024） |
| MySQL | 8.0+（开发于 8.4，utf8mb4） |
| Node / pnpm | Node 20+ / pnpm 9+ |
| nginx | 任意近期版本（仅生产部署需要） |

### 快速开始

**1. 建库**（表由服务启动时自动创建，无需手工建表）

```sql
CREATE DATABASE bee_admin DEFAULT CHARSET utf8mb4 COLLATE utf8mb4_0900_ai_ci;
```

**2. 配置**

```bash
cp admin/conf/app.conf.example admin/conf/app.conf
# 必填三项：
#   [db] dsn                              MySQL 账号密码
#   [jwt] secret                          ≥32 字符随机串，且不能是 changeme
#   [app] encrypt_key                     base64:<32 字节>，生成：openssl rand -base64 32 | sed 's/^/base64:/'
# 建议两项（部署间必须不同）：
#   [app] hashids_salt                    对外 id 短串的盐
#   [app] snowflake_worker / snowflake_dc 多实例各进程必须不同，否则撞号
```

> ⚠️ `encrypt_key` **缺失时服务拒绝启动**（刻意的：不允许「以为加密了其实没加密」地跑起来），
> 且它**不可更换** —— 换了密钥，库里已加密的邮箱/手机号就解不开了。
> 同理 `hashids_salt` 一换，已发出去的短串 id 全部作废。

其余可选开关见示例文件：`[log] retain_days`（日志保留天数，`0` = 永久，同时作用于
登录/操作/任务执行记录）、`[job] enabled`（定时任务总开关，关掉后仍可手动补跑）、
`[auth] max_fail` / `lock_minutes`（登录失败锁定）、`[auth] captcha`（登录图形验证码，
默认开）。

> 升级已有部署时：验证码凭据由新前端提交。**若先换后端、暂不重建前端，先把
> `[auth] captcha` 写成 `false`**，否则老前端的登录会被判 400（验证码错误）。

**3. 起服务**（首次启动自动建表、写入超管与菜单权限种子）

```bash
cargo run -p bee_admin
curl http://127.0.0.1:8080/api/v1/health   # => OK
```

**4. 起前端**

```bash
cd admin/web && pnpm i && pnpm dev
# 打开 http://127.0.0.1:5173（vite 已把 /api 代理到 127.0.0.1:8080）
```

也可以用生产构建在本机预览（端口与线上 nginx 一致，同样反代 `/api`）：

```bash
cd admin/web && pnpm build && pnpm preview   # http://localhost:8081
```

### 初始账号

| 账号 | 密码 |
|---|---|
| `admin` | `admin/conf/app.conf` 的 `[seed] initial_admin_password`（示例配置为 `admin123`） |

超管仅在首次启动（`admin` 表为空）时写入。**登录后请立即到「个人中心」修改密码**；
生产环境务必同时改掉 `initial_admin_password` 与 `[jwt] secret`。

> 提示：`BEE_ADMIN_CONF` 可指定配置文件路径；`BEE_ADMIN_DB_DSN` 覆盖数据库连接串；
> `BEE_ADMIN_HTTP_ADDR` 覆盖监听地址（容器里绑 `0.0.0.0:8080` 时必须用）。

### 接口文档站（apidoc-rust）

后台内置 apidoc-rust 的接口文档站，**默认关闭**（文档页会把整个后台的接口面、请求/响应
结构都摊开，它是开发/交付工具，不该跟着生产实例一起对外）。要开就在 `app.conf` 里写：

```ini
[apidoc]
enabled = true
password = <自己定一个口令>
```

- `enabled = true` 而 `password` 空着 → **拒绝启动**（与 `encrypt_key` 一个态度：不允许
  「以为有口令其实没有」地跑起来）；非法布尔值同样拒绝启动
- 挂载点：`/apidoc`（UI，浏览器直接开）、`/apidoc/api.json`（原始接口清单，64 个接口）
- 文档站**不在审计 / 安全扫描层的覆盖范围内**（它 merge 在全部中间件之后，读文档不是业务
  操作，记进操作日志只是噪音），整棵树也在 `/api/v1` 之外，与业务接口互不影响

数据路由（`api.json` / `export` / `mock` / `share` / `generate`）要带 token：先拿口令的
md5 去换 token，再把 token 带在 query 上（token 里有 `+` `/` 等字符，要 URL 编码）：

```bash
MD5=$(printf '%s' '你的口令' | md5sum | cut -d' ' -f1)   # 收尾别带换行
TOKEN=$(curl -s "http://127.0.0.1:8080/apidoc/auth?password=$MD5" | sed 's/.*"token":"\([^"]*\)".*/\1/')
curl -G --data-urlencode "token=$TOKEN" http://127.0.0.1:8080/apidoc/api.json
```

`/apidoc`（UI 页本身）不设守卫，口令只在取 token 时校验 —— 别把 UI 当成「有密码的站点」。

> ⚠️ **插件那套口令的凭据是 `md5(口令)` 放在 query 上，等价于一个可重放的通行证** ——
> URL 在访问日志 / 浏览器历史 / 跳板机记录里留一次，抓到的人就能一直换 token；且 md5 无盐、
> 弱口令可离线爆破，插件默认的 `secret_key` 又是公开常量。所以**只在可信网络（内网 /
> 跳板机）里开**，别对着公网。

### Docker 一键部署

仓库里已备好容器化配置：

| 文件 | 作用 |
|---|---|
| `Dockerfile` | 后端镜像：多阶段构建（Rust 编译 → Debian 运行），非 root 用户运行 |
| `admin/web/Dockerfile` | 前端镜像：pnpm 构建 → nginx，`/api/` 上游由环境变量 `BACKEND_UPSTREAM` 注入 |
| `docker-compose.yml` | MySQL + 后端 + 前端一键起栈，对外 `8081` |

```bash
cp .env.example .env && vi .env                      # 填 MySQL root 密码
cp admin/conf/app.conf.example admin/conf/app.conf   # 填 [jwt] secret（≥32 字符）
docker compose up -d --build
# 打开 http://localhost:8081（初始账号 admin / 配置里的 initial_admin_password）
```

- 容器内的数据库地址由 `BEE_ADMIN_DB_DSN` 注入（指向 compose 里的 `mysql` 服务）；
  配置文件仍挂载自 `admin/conf/app.conf`，JWT secret 与初始密码从那里读
- 后端容器以**宿主机配置文件属主**的身份运行（`APP_UID`/`APP_GID`，默认 `1000:1000`）：
  `app.conf` 通常是 600 权限，用镜像默认的非 root 用户会读不到配置而启动失败；
  宿主 uid 不是 1000 时在 `.env` 里改
- 想接已有的 MySQL：删掉 compose 里的 `mysql` 服务与 `depends_on`，直接改 `BEE_ADMIN_DB_DSN`
- 只构建镜像（不启动）：`docker build -t bra-backend .`（后端）与
  `docker build -t bra-web admin/web`（前端）
- 镜像只在本机构建/离线分发，**不经过 GitHub CI 发布**；需要私有镜像仓库时用
  `docker tag` + `docker push` 推到自己的仓库即可

### 测试

```bash
# 框架：单测 + 真库集成（未设 DSN 的集成测试跳过并打印原因）
cargo test -p bee_orm
BEE_ORM_TEST_DSN='mysql://user:pass@127.0.0.1:3306/bee_orm_test' cargo test -p bee_orm -- --nocapture

# 后台：单测（配置 / JWT / 数据权限 / 防环等纯函数）
cargo test -p bee_admin --bins

# 后台：整包（单测 + 全部真库集成套件；不逐个列 --test，避免新套件被漏在 CI 外）
# 注意：会先 DROP 测试库所有表再重建，只对测试库执行
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  cargo test -p bee_admin -- --nocapture
```

`BEE_ADMIN_DB_DSN` 会覆盖配置里的 `[db] dsn`（便于测试注入凭据），因此测试库账号不必写进仓库；
集成测试带**生产库护栏**——DSN 指向的库名不以 `_test` 结尾时直接拒绝执行。

### 部署

systemd 托管服务（监听 `127.0.0.1:8080`）+ nginx 对外 8081：静态资源指向前端构建产物
`admin/web/dist`，`/api/` 反向代理到后端并透传 `X-Real-IP`（登录记录需要真实 IP）。

```bash
cargo build --release -p bee_admin
cd admin/web && pnpm build

sudo cp admin/deploy/bee-admin.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now bee-admin

sudo cp admin/deploy/nginx.conf.example /usr/local/nginx/conf/vhost/bee-admin.conf
sudo nginx -t && sudo systemctl reload nginx
```

文件内的注释写了各自的安装位置与前置条件；设计文档 §7 有部署方案的完整说明。

## 版权

© 2026 erik · <https://erik.xyz>

本项目基于 [Apache-2.0](LICENSE) 许可发布。
