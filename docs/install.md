# BRA 安装说明

Bee Rust Admin —— Rust 后端 `bee_admin` + React 前端 dist + MySQL 8 + nginx。
本文给部署者：讲清怎么装。装好之后怎么用，见 [使用手册](usage.md)；排障与运维见 [常见问题与运维](faq.md)。

---

## 0. 前置要求

| 组件 | 要求 | 说明 |
|---|---|---|
| 系统 | **Linux x86_64** | ARM 只能从源码构建或用 Docker（没有 arm64 预编译产物） |
| 数据库 | **MySQL 8.0+**，字符集 `utf8mb4` | 表结构与超管账号由服务**首次启动时自动创建**，不用手工导 SQL |
| glibc | **≥ 2.38** | 只有用预编译产物才要求，见下 |
| 权限 | root 或 sudo | **只有**装 systemd 单元 / nginx 站点那一步需要，其余步骤普通用户即可 |

### ⚠️ 最要紧的一条：glibc 版本

预编译二进制需要 **glibc ≥ 2.38**（= Debian 13 / Ubuntu 24.04 及以上）。
Ubuntu 22.04（2.35）、Debian 12（2.36）、CentOS 上会直接报
`version 'GLIBC_2.38' not found`，二进制根本跑不起来。

```bash
ldd --version | head -1     # 末尾数字就是 glibc 版本
```

**低于 2.38 就分叉**：改用「从源码构建」（`--mode build`）或「Docker」，别看一键脚本了。

> `install.sh` 自己会检测这件事 —— 它会让下载来的二进制**真的加载一次**，
> 跑不起来就在**写盘之前**退出，并给出上面这两条替代命令，
> 不会留下一个装了一半、又起不来的安装。

---

## 1. 生成配置

`app.conf` 里有三样密钥必须自己生成（**别用示例文件里的占位符，服务会拒绝启动**）：

```bash
# 数据库口令（明文也行，脚本/示例里的都会处理）
# 落库加密密钥（AES-256-GCM）—— 必须是 32 字节
echo "encrypt_key = base64:$(openssl rand -base64 32)"
# 对外短 id 的盐
echo "hashids_salt = $(openssl rand -hex 16)"
# JWT 签名密钥（≥32 字符）
echo "secret = $(openssl rand -hex 32)"
```

然后把下面这份**改好填进 `<prefix>/conf/app.conf`**（权限 `600`）：

```ini
[app]
name = bee-rust-admin
http_addr = 127.0.0.1:8080
log_level = info
upload_dir = uploads
hashids_salt = <上面生成的 hex>
hashids_min_len = 8
snowflake_worker = 0
snowflake_dc = 0
encrypt_key = base64:<上面生成的那串>

[db]
dsn = mysql://<用户>:<口令>@127.0.0.1:3306/bee_admin

[jwt]
secret = <上面生成的 hex>
expire_hours = 24

[auth]
max_fail = 5
lock_minutes = 10
captcha = true
captcha_trajectory = false

[log]
retain_days = 90

[job]
enabled = true

[security]
scan = off

[apidoc]
enabled = false
password =

[seed]
initial_admin_password = <自己定一个>
```

> ⚠️ **`encrypt_key` 与 `hashids_salt` 生成后不可更换**：
> 换 `encrypt_key` = 库里已加密的 email/phone **永久解不开**（没有救援）；
> 换 `hashids_salt` = 已发出的对外短 id 全部作废。**生成完就把这份 conf 备份好。**

> ⚠️ `initial_admin_password` 只在**首次启动（admin 表为空）**时用来创建内置超管 `admin`：
> 太弱（不足 8 位、或少于字母/数字/符号中的两类）会在**那一次**启动时被拒绝并给出明确错误，
> 库非空后这个键就再也不起作用了。校验**刻意不放在配置加载期** —— 免得存量部署
> （conf 里还留着老口令）在换值之前起不来。

各键的完整说明（含默认值与「多实例必须各不同」之类）见
[`admin/conf/app.conf.example`](../admin/conf/app.conf.example)。

---

## 2. 三种装法

### (a) 一键脚本（推荐）

**下载仓库后跑，别直接 `curl | bash`** —— 它要 root 装 systemd 与 nginx，值得先看一眼；
而且 `install.sh` 只是入口，具体步骤在同仓库的 `scripts/lib/install-*.sh`（缺了会直接报错退出）。
**旧版「只下载一个 `install.sh` 再 `curl | bash`」的用法已退役** —— 单文件跑不起来，必须先克隆完整仓库：

```bash
git clone --depth 1 https://github.com/erikwang2013/bee-rust-admin.git
cd bee-rust-admin
less install.sh          # 先看一眼（步骤在 scripts/lib/install-*.sh）
bash install.sh          # 交互式：会问数据库口令
```

它依次做：拉发布产物（二进制 + 前端 dist）→ 生成密钥 → 写 `<prefix>/conf/app.conf`（0600）
→ 建库（可选）→ 装 systemd 单元 + nginx 站点 → 起服务并自检 `/api/v1/health`。
默认装到 `/opt/bee-admin`，对外 `8081`。

- **建库是可选的**：给 `--db-admin-user U --db-admin-pass P` 它才自动建库建号；
  不给就只打印 SQL 让你手工执行（它不猜 root 口令、不 `DROP`、不装 MySQL）。
  **升级（`--keep-conf`）时不建库建号**：连库凭据以已有 `app.conf` 里的 dsn 为准，`--db-*` 不参与；
  连不上会退出并让你自己修 dsn（它绝不 ALTER 库里已有的账号）
- 表结构与超管账号由服务**首次启动时自动创建**，不需要手工导 SQL

常用变体（参数名照 `install.sh --help`）：

```bash
# 老机器（glibc < 2.38）：从源码构建（需要 rust 1.99+ / node 22.12+ / pnpm 10+）
bash install.sh --mode build

# 装第二套实例（目录、服务名、端口、雪花节点全错开）
bash install.sh --service-name bee-admin-2 --prefix /opt/bee-admin-2 \
                --http-port 8091 --backend-port 8090 --snowflake 2,0

# 先看会做什么（不落盘、不下载、不连库）
bash install.sh --dry-run --yes

# 升级：只换二进制与服务文件，配置一个字节都不动（--db-* 不参与，DSN 以已有 conf 为准）
bash install.sh --keep-conf ...

# 没有 root / 自己管进程：不装 systemd 单元（也不会去 restart 服务，装完打印手动启动命令），
# 站点也不装就再加 --no-nginx
bash install.sh --no-systemd --no-nginx
```

> **已经有 `app.conf` 时，脚本默认拒绝覆盖**（退出码 `6`）—— 因为 `encrypt_key` 一换，
> 库里已加密的 email/phone 就永久解不开。要覆盖得显式加 `--force`：它会先把原文件备份到
> `app.conf.bak.<时间戳>`，但换了密钥之后数据仍可能解不开 —— 只有明确知道后果才用它。
> **正常升级走 `--keep-conf`。**

**退出码**：`0` 成功 · `1` 参数错误 / 构建失败等其它失败 · `2` 预检失败（什么都没写）· `3` 已就绪、只差 root 执行打印的命令 ·
`4` 需要人工建库 · `5` 自检失败 · `6` 检测到已有配置且未给 `--force`。

### (b) Docker compose

仓库里已备好单机 compose 部署（MySQL + 一次性初始化 + 后端 + 前端 nginx，各服务独立
容器）。`compose-up.sh` 负责生成/迁移密钥、渲染 `.env` 与 `admin/conf/app.conf`，再起栈：

```bash
./compose-up.sh --build     # 从源码构建并起栈；已有镜像时去掉 --build
# 默认 HTTP: http://localhost:8081；初始账号 admin
# 初始口令在 secrets/initial_admin_password（也渲染进 app.conf 的初始口令项）
```

- **密钥在本地**：`secrets/`（0600）保存 MySQL root/应用口令、JWT secret、`encrypt_key`、
  `hashids_salt`、初始管理员口令；首次缺失自动生成，已存在不覆盖。重生成用
  `./compose-up.sh --regen <name>`（`all` 全部），旧值在 `secrets/backups/`。
  注意 `encrypt_key` 更换后库里 email/phone 解不开，MySQL 口令重新生成后已有库需手动改库
- **数据和日志不在容器层**：业务数据在宿主机 `data/bra/`（挂后端 `/home/uploads`），
  MySQL 数据在命名卷；日志用 Docker json-file 轮转（10m×5），后端 /tmp、nginx 缓存用 tmpfs；
  nginx 对 `/api/` 的请求与响应都不做缓冲，导入/导出直接流式转发，不会撑满 32MB tmpfs
- **域名证书唯一手工配置**：`certs/fullchain.pem` + `certs/privkey.pem`，`.env` 配
  `SERVER_NAME`；证书齐全时 `./compose-up.sh` 自动加 `docker-compose.tls.yml` 走 80/443，
  否则 HTTP `:8081`
- 后端以 `.env` 的 `APP_UID/APP_GID` 运行；新建 `.env` 时自动取当前用户（旧 `.env`
  的显式值保留），root 只在属主不一致时把 app.conf 与 `data/bra` 改成该 uid。要换用户就在
  `.env` 显式改，否则读不到 600 的 app.conf、也写不了 `data/bra`
- Docker 镜像包含 `mariadb-client` 的 `mysqldump`，内置 `db_backup` 开箱可用；裸机部署
  需要自己安装 MySQL 客户端
- 上传中断留下的 `_header/`、`.part` 与中断备份的 `backup_*.sql.part` 超过 24 小时会由
  内置清理任务回收，不会一直堆在 `data/bra/`
- 升级旧 compose 部署：先把旧容器 `/home/uploads` 拷到 `data/bra/`（重建后容器层会丢），
  旧 `.env`/app.conf 的密钥会自动迁移到 `secrets/`，包括只存在于旧 `[db] dsn` 里的外部
  MySQL 口令；旧版 TLS 残留的 `HTTP_PORT=80` 在 `--http` 启动时会复位为 8081
- 离线包：Release 附带 `bra-images-vX.Y.Z.tar.gz` 与 `bra-compose-vX.Y.Z.tar.gz`；解包、
  `docker load -i`、`./compose-up.sh` 即可，证书按 `certs/README.md` 放好

### (c) 手工部署

systemd 托管后端（监听 `127.0.0.1:8080`）+ nginx 对外 `8081`：

```bash
cargo build --release -p bee_admin
cd admin/web && pnpm build

sudo cp admin/deploy/bee-admin.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now bee-admin

# 站点目录按自己的 nginx 来：自编译版常在 <prefix>/conf/vhost/，发行版在 /etc/nginx/conf.d/
sudo cp admin/deploy/nginx.conf.example /usr/local/nginx/conf/vhost/bee-admin.conf
sudo nginx -t && sudo systemctl reload nginx
```

> `admin/deploy/` 里那两个文件里的路径与用户名是**示例**（写的是
> `/home/wwwroot/bee-rust-admin` 和 `erik`），要按自己的改成实际值 ——
> 或者直接用 `install.sh`，它会按实际路径 / 用户 / 端口渲染这两份文件。

**没有 root / 不想 sudo 的机器用「用户级单元」**，能力与上面系统级等价：

| | 系统级 | 用户级（免 root） |
|---|---|---|
| 单元文件 | `admin/deploy/bee-admin.service` | `admin/deploy/bee-admin.user.service` |
| 安装 | `sudo cp` 到 `/etc/systemd/system/` | `cp` 到 `~/.config/systemd/user/`，不用 sudo |
| 开机自启 | 直接支持 | 需要一次性 `sudo loginctl enable-linger <用户>`（`loginctl show-user <用户> -p Linger` 查；没开就等该用户登录后才起） |
| 管理 / 日志 | `systemctl status/restart bee-admin`、`journalctl -u bee-admin` | 同样命令**全部带 `--user`**（root 的 `systemctl status` 看不到用户级单元） |
| 崩溃重启 | `Restart=on-failure`，3 秒重试 | 同左 |

```bash
# 用户级安装（免 root；路径示例即本机实际值）
mkdir -p ~/.config/systemd/user
cp admin/deploy/bee-admin.user.service ~/.config/systemd/user/bee-admin.service
systemctl --user daemon-reload && systemctl --user enable --now bee-admin
```

日常（升级 = 重建二进制后重启即生效）：

```bash
systemctl --user status bee-admin          # 系统级就去掉 --user
systemctl --user restart bee-admin
journalctl --user -u bee-admin -f          # 跟日志
```

> ⚠️ 两份单元模板的 `WorkingDirectory` 不同（系统级示例是 `<仓库>/admin`，用户级是**仓库根**）——
> `uploads` 是**相对 WorkingDirectory** 落盘的，**迁移已有部署时保持原 cwd**，换了头像就 404。

---

## 3. 装完自检

```bash
curl http://127.0.0.1:8080/api/v1/health     # => OK
```

然后浏览器打开 `http://<这台机器的地址>:8081`（端口按你改的替换），
用 `admin` + `[seed] initial_admin_password` 登录，**登录后立刻改密码**。

---

## 4. 常见坑

- **`app.conf` 是 600 权限**：跑服务的用户必须是它的属主，否则起不来；容器里则要对上 `APP_UID`/`APP_GID`
- **`encrypt_key` 不可更换**：换 = 库里 email/phone 永久解不开（没有救援）。备份它，连同整份 `app.conf` 一起
- **`hashids_salt` 一换**，已发出去的对外短 id 全部作废
- **初始口令只在库为空时生效**：库里已有 `admin` 就忽略它，得登录后在后台改
- **前后端要一起换**：升级时后端删了旧的 base64 头像接口，老前端配新后端会在头像上传上报 404
- **验证码默认开**（`[auth] captcha = true`）：登录要多点一下；答错不计入锁定次数
- **拖拽轨迹校验默认关**（`[auth] captcha_trajectory`）：开启后滑块答案必须带轨迹
  （插件默认阈值：≥4 点、300–5000ms、线性度 ≤0.99），只交裸数值的老客户端一律失败。
  新前端总是上报轨迹，开这个开关不用重发前端；代价是「极快甩动」（<300ms）会被判机器 ——
  用户表现为滑块怎么拖都提示「验证码错误」，要放宽得改插件侧的 `min_duration_ms`
- **仓库根目录的 `install.sql` 与本项目无关**（那是另一个工具的库），安装不需要它
- **忘了超管密码**：改库里的 `admin` 表（或重建库重来）
