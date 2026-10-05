# bee-rust 管理后台

基于本仓库的 bee-rust 框架（axum + sqlx ORM + INI 配置）实现的管理后台：
JWT 登录、管理员 / 角色 / 菜单 / 部门 / 登录记录管理，菜单 + 按钮权限 + 部门数据权限，
前端 React + Vite + Ant Design 5。

- 服务端：`admin/`（crate `bee_admin`，默认监听 `127.0.0.1:8080`）
- 前端：`admin/web/`（开发 5173，生产由 nginx 8081 提供）
- 设计文档：`docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md`

## 目录结构

```
crates/            bee-rust 框架（含本后台依赖的 bee_orm 执行层、bee_router、bee_config）
admin/             管理后台服务端（crate bee_admin）
  src/api/         接口：auth / admin / role / menu / dept / login_log
  src/models/      模型（#[derive(Model)]，表结构由 syncdb 生成）
  src/datascope.rs 数据权限解析（全部/本部门及以下/本部门/仅本人/自定义）与查询注入
  src/seed.rs      建表（syncdb）+ 首次启动种子（超管 + 菜单权限树）
  conf/            app.conf（gitignore）/ app.conf.example
  deploy/          systemd 单元 + nginx 站点配置示例
  tests/           集成测试（起真实进程 + 真库）
  web/             前端（React + antd）
docs/              设计文档与实施计划
```

## 快速开始

1. **建库**（MySQL 8.4，utf8mb4；表由服务启动时自动创建，无需手工建表）：

   ```sql
   CREATE DATABASE bee_admin DEFAULT CHARSET utf8mb4 COLLATE utf8mb4_0900_ai_ci;
   ```

2. **配置**：

   ```bash
   cp admin/conf/app.conf.example admin/conf/app.conf
   # 填 [db] dsn（MySQL 账号密码）与 [jwt] secret（≥32 字符随机串，且不能是 changeme）
   ```

3. **起服务**（首次启动自动建表并写入超管、菜单权限种子）：

   ```bash
   cargo run -p bee_admin
   curl http://127.0.0.1:8080/api/v1/health   # => OK
   ```

4. **起前端**：

   ```bash
   cd admin/web && pnpm i && pnpm dev
   # 打开 http://127.0.0.1:5173（vite 已把 /api 代理到 127.0.0.1:8080）
   ```

## 初始账号

| 账号 | 密码 |
|---|---|
| `admin` | `admin/conf/app.conf` 的 `[seed] initial_admin_password`（示例配置为 `admin123`） |

超管仅在首次启动（admin 表为空）时写入。**登录后请立即到「个人中心」修改密码**；
生产环境务必同时改掉 `initial_admin_password` 与 `[jwt] secret`。

## 测试

```bash
# 框架：单测 + 真库集成（未设 DSN 的集成测试跳过并打印原因）
cargo test -p bee_orm
BEE_ORM_TEST_DSN='mysql://user:pass@127.0.0.1:3306/bee_orm_test' cargo test -p bee_orm -- --nocapture

# 后台：单测（配置 / JWT / 数据权限 / 防环等纯函数）
cargo test -p bee_admin --bins

# 后台全链路：真实进程 + 真库，覆盖登录、五个模块 CRUD、数据权限、踢下线、引用校验
# 注意：会先 DROP 测试库所有表再重建，只对测试库执行
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  cargo test -p bee_admin --test api_flow_test -- --nocapture
```

`BEE_ADMIN_DB_DSN` 会覆盖配置里的 `[db] dsn`（便于测试注入凭据），因此测试库账号不必写进仓库。

## 部署

systemd 托管服务（监听 `127.0.0.1:8080`）+ nginx 对外 8081：静态资源指向前端构建产物
`admin/web/dist`，`/api/` 反向代理到后端并透传 `X-Real-IP`（登录记录需要真实 IP）。

```bash
cargo build --release -p bee_admin
cd admin/web && pnpm build
sudo cp admin/deploy/bee-admin.service /etc/systemd/system/ && sudo systemctl daemon-reload && sudo systemctl enable --now bee-admin
sudo cp admin/deploy/nginx.conf.example /usr/local/nginx/conf/vhost/bee-admin.conf && sudo nginx -t && sudo systemctl reload nginx
```

文件内的注释写了各自的安装位置与前置条件；设计文档 §7 有部署方案的完整说明。
