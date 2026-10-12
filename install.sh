#!/usr/bin/env bash
# BRA（Bee Rust Admin）一键安装脚本。
#
#   bash install.sh                     # 交互式（会问数据库口令等），装到 /opt/bee-admin
#   bash install.sh --help              # 全部参数
#   bash install.sh --dry-run --yes     # 只打印将要写的一切，不落盘
#
# 设计要点（改这个脚本前先读）：
#
# 1. **默认 download 模式**：从 GitHub Releases 拉预编译产物（二进制 + 前端 dist），
#    零工具链依赖（不要 rust、不要 node）。--mode build 才从源码构建。
# 2. **预编译产物需要 glibc 2.38**（Debian 13 / Ubuntu 24.04+）。装之前会让二进制
#    自己加载一次来判定 —— 加载不了就在**写盘之前**退出并给两条逃生路径，
#    绝不留下一个装了一半、跑不起来的安装。
# 3. **绝不覆盖已有的 app.conf**。它不是普通配置文件：`encrypt_key` 换了，
#    库里已加密的 email/phone 就永久解不开；`hashids_salt` 换了，已发出去的对外
#    短串全作废。默认行为是拒绝（退出 6），要覆盖必须显式 `--force`（先备份）。
# 4. **只写 <prefix> 与（拿到 root 时）systemd/nginx 的安装位置**。不碰仓库、
#    不碰别人的服务、不 DROP 任何库。
#
# 退出码：0 成功 · 1 通用失败 · 2 预检失败（未写任何东西）· 3 已就绪，只差 root 执行
#         打印的命令 · 4 需要人工建库 · 5 自检失败 · 6 检测到已有配置且未给 --force
#
# 注意：仓库根目录的 install.sql 与本项目**无关**（那是另一个工具的 SQLite 库），
#       别被名字骗到，安装不需要也不使用它。
set -euo pipefail
umask 022
ORIG_ARGS=("$@")   # 参数解析会把 $@ 吃光，后面要拿它拼「重跑」提示

PROG=${0##*/}
EXIT_OK=0; EXIT_FAIL=1; EXIT_PREFLIGHT=2; EXIT_NEED_ROOT=3
EXIT_NEED_DB=4; EXIT_SELFTEST=5; EXIT_CONF_EXISTS=6

# ══════════════════════════════════════════════════════════════════════════
#  输出
# ══════════════════════════════════════════════════════════════════════════
if [ -t 2 ]; then
  C_RED=$'\033[31m'; C_YEL=$'\033[33m'; C_GRN=$'\033[32m'; C_BLD=$'\033[1m'; C_OFF=$'\033[0m'
else
  C_RED=''; C_YEL=''; C_GRN=''; C_BLD=''; C_OFF=''
fi
say()  { printf '%s==>%s %s\n' "$C_BLD" "$C_OFF" "$*" >&2; }
info() { printf '    %s\n' "$*" >&2; }
warn() { printf '%s警告:%s %s\n' "$C_YEL" "$C_OFF" "$*" >&2; }
die()  { local code=$1; shift; printf '%s错误:%s %s\n' "$C_RED" "$C_OFF" "$*" >&2; exit "$code"; }
step_fail() { local code=$1; shift; printf '\n%s[失败]%s %s\n' "$C_RED" "$C_OFF" "$*" >&2; exit "$code"; }

usage() {
  cat <<'EOF'
BRA 一键安装脚本

用法：bash install.sh [参数]

安装方式
  --mode download|build   产物来源。download（默认）= 拉 GitHub Releases 的预编译产物；
                          build = 从源码构建（需要 rust 1.99+ / node 20+ / pnpm 9+）
  --version TAG           要装的版本（download 模式）。默认 latest（查 GitHub）

位置与命名
  --prefix DIR            安装根目录（默认 /opt/bee-admin）
  --service-name NAME     systemd 单元名 / nginx vhost 文件名（默认 bee-admin）
                          装第二套实例时靠它 + --prefix + 端口区分
  --user U / --group G    服务以什么身份跑（默认：root 时为 $SUDO_USER，否则当前用户）
  --repo DIR              源码目录（仅 --mode build；默认取脚本所在目录）
  --base-url URL          产物下载地址前缀，支持 file://（内网镜像 / 离线验证用）
                          默认 https://github.com/erikwang2013/bee-rust-admin/releases/download/<TAG>

端口
  --http-port N           对外端口（nginx listen，默认 8081）
  --backend-port N        后端监听端口（默认 8080，只被 nginx 反代，不对外）

数据库
  --db-host H             默认 127.0.0.1
  --db-port N             默认 3306
  --db-name N             默认 bee_admin
  --db-user U             默认 bee_admin（应用账号，不是 root）
  --db-pass P             应用账号口令。不给则随机生成并打印
  --db-admin-user U       给了才**自动建库建号**（需要 CREATE USER 权限的账号）
  --db-admin-pass P       同上。不给的话脚本只打印 SQL 让你手工执行

密钥与账号（不给就随机生成；从旧库迁移时可显式对齐）
  --encrypt-key K         [app] encrypt_key，格式 base64:<32字节>
                          ⚠️ 不可更换：换了库里已加密的 email/phone 全部解不开
  --jwt-secret S          [jwt] secret，≥32 字符
  --hashids-salt S        [app] hashids_salt，换了已发出的对外短串全部作废
  --snowflake W,D         雪花节点号,数据中心号（默认 0,0；多实例必须各不相同）
  --admin-password P      超管初始口令，默认随机。**只在库为空（首次启动）时生效**
  --app-name N            [app] name（默认 bee-rust-admin）

已有安装 / 危险操作
  --keep-conf             复用已有的 app.conf（升级路径）：只换二进制与配置好的服务文件，
                          配置文件一个字节都不动
  --force                 允许覆盖已有 app.conf。**会先备份**，但换了 encrypt_key 之后
                          库里数据仍可能解不开 —— 只有明确知道后果才用它
  --nginx-conf-dir DIR    显式指定 nginx 站点目录（默认自动探测）

其它
  --no-systemd            不装 systemd 单元，只渲染到 <prefix>/etc/ 并打印要执行的命令
  --no-nginx              同上（不装 nginx 站点）
  --dry-run               只打印将要写的一切，不落盘、不下载、不连库
  --yes, -y               不交互，全部用默认值/参数
  --help                  显示本帮助

示例
  # 最省事：交互式装一套（会问数据库口令）
  bash install.sh

  # 老机器（glibc < 2.38）从源码构建
  bash install.sh --mode build

  # 装第二套实例（端口与目录都错开）
  bash install.sh --service-name bee-admin-2 --prefix /opt/bee-admin-2 \
                  --http-port 8091 --backend-port 8090 --snowflake 2,0

  # 先看会做什么，什么都不写
  bash install.sh --dry-run --yes
EOF
}

# ══════════════════════════════════════════════════════════════════════════
#  参数解析
# ══════════════════════════════════════════════════════════════════════════
MODE=${BRA_MODE:-download}
VERSION=${BRA_VERSION:-latest}
BASE_URL=${BRA_BASE_URL:-}
PREFIX=${BRA_PREFIX:-/opt/bee-admin}
SERVICE_NAME=${BRA_SERVICE_NAME:-bee-admin}
RUN_USER=${BRA_USER:-}
RUN_GROUP=${BRA_GROUP:-}
REPO_DIR=${BRA_REPO:-}
HTTP_PORT=${BRA_HTTP_PORT:-8081}
BACKEND_PORT=${BRA_BACKEND_PORT:-8080}
DB_HOST=${BRA_DB_HOST:-127.0.0.1}
DB_PORT=${BRA_DB_PORT:-3306}
DB_NAME=${BRA_DB_NAME:-bee_admin}
DB_USER=${BRA_DB_USER:-bee_admin}
DB_PASS=${BRA_DB_PASS:-}
DB_ADMIN_USER=${BRA_DB_ADMIN_USER:-}
DB_ADMIN_PASS=${BRA_DB_ADMIN_PASS:-}
ENCRYPT_KEY=${BRA_ENCRYPT_KEY:-}
JWT_SECRET=${BRA_JWT_SECRET:-}
HASHIDS_SALT=${BRA_HASHIDS_SALT:-}
SNOWFLAKE=${BRA_SNOWFLAKE:-0,0}
ADMIN_PASSWORD=${BRA_ADMIN_PASSWORD:-}
APP_NAME=${BRA_APP_NAME:-bee-rust-admin}
KEEP_CONF=0; FORCE=0; WITH_SYSTEMD=1; WITH_NGINX=1; DRY_RUN=0; ASSUME_YES=0
NGINX_CONF_DIR=${BRA_NGINX_CONF_DIR:-}

SCRIPT_DIR=$(cd -- "$(dirname -- "$0")" 2>/dev/null && pwd -P || echo "")
# curl | bash 时没有仓库；download 模式本来也不需要
[ -z "$REPO_DIR" ] && REPO_DIR=$SCRIPT_DIR

need_val() { [ "$2" -ge 1 ] || die "$EXIT_FAIL" "$1 需要一个值（--help 看用法）"; }
while [ $# -gt 0 ]; do
  case $1 in
    --mode)            need_val "$1" $#; MODE=$2; shift 2;;
    --version)         need_val "$1" $#; VERSION=$2; shift 2;;
    --base-url)        need_val "$1" $#; BASE_URL=$2; shift 2;;
    --prefix)          need_val "$1" $#; PREFIX=$2; shift 2;;
    --service-name)    need_val "$1" $#; SERVICE_NAME=$2; shift 2;;
    --user)            need_val "$1" $#; RUN_USER=$2; shift 2;;
    --group)           need_val "$1" $#; RUN_GROUP=$2; shift 2;;
    --repo)            need_val "$1" $#; REPO_DIR=$2; shift 2;;
    --http-port)       need_val "$1" $#; HTTP_PORT=$2; shift 2;;
    --backend-port)    need_val "$1" $#; BACKEND_PORT=$2; shift 2;;
    --db-host)         need_val "$1" $#; DB_HOST=$2; shift 2;;
    --db-port)         need_val "$1" $#; DB_PORT=$2; shift 2;;
    --db-name)         need_val "$1" $#; DB_NAME=$2; shift 2;;
    --db-user)         need_val "$1" $#; DB_USER=$2; shift 2;;
    --db-pass)         need_val "$1" $#; DB_PASS=$2; shift 2;;
    --db-admin-user)   need_val "$1" $#; DB_ADMIN_USER=$2; shift 2;;
    --db-admin-pass)   need_val "$1" $#; DB_ADMIN_PASS=$2; shift 2;;
    --encrypt-key)     need_val "$1" $#; ENCRYPT_KEY=$2; shift 2;;
    --jwt-secret)      need_val "$1" $#; JWT_SECRET=$2; shift 2;;
    --hashids-salt)    need_val "$1" $#; HASHIDS_SALT=$2; shift 2;;
    --snowflake)       need_val "$1" $#; SNOWFLAKE=$2; shift 2;;
    --admin-password)  need_val "$1" $#; ADMIN_PASSWORD=$2; shift 2;;
    --app-name)        need_val "$1" $#; APP_NAME=$2; shift 2;;
    --nginx-conf-dir)  need_val "$1" $#; NGINX_CONF_DIR=$2; shift 2;;
    --keep-conf)       KEEP_CONF=1; shift;;
    --force)           FORCE=1; shift;;
    --no-systemd)      WITH_SYSTEMD=0; shift;;
    --no-nginx)        WITH_NGINX=0; shift;;
    --dry-run)         DRY_RUN=1; shift;;
    --yes|-y)          ASSUME_YES=1; shift;;
    --help|-h)         usage; exit "$EXIT_OK";;
    -*)                die "$EXIT_FAIL" "未知参数：$1（--help 看用法）";;
    *)                 die "$EXIT_FAIL" "多余的位置参数：$1（--help 看用法）";;
  esac
done

case $MODE in download|build) ;; *) die "$EXIT_FAIL" "--mode 只能是 download 或 build，给的是 $MODE";; esac
case $SNOWFLAKE in [0-9]*,[0-9]*) ;; *) die "$EXIT_FAIL" "--snowflake 形如 0,0（节点号,数据中心号）";; esac
case $DB_NAME in *[!A-Za-z0-9_]*) die "$EXIT_FAIL" "--db-name 只允许字母数字下划线（它要拼进 SQL）";; esac
for p in "$HTTP_PORT" "$BACKEND_PORT" "$DB_PORT"; do
  case $p in *[!0-9]*|'') die "$EXIT_FAIL" "端口必须是数字，给的是：$p";; esac
done

PREFIX=${PREFIX%/}
CONF=$PREFIX/conf/app.conf
STAGE_ETC=$PREFIX/etc
UNIT_FILE=$STAGE_ETC/$SERVICE_NAME.service
VHOST_FILE=$STAGE_ETC/nginx-$SERVICE_NAME.conf
VERSION_FILE=$PREFIX/VERSION

GITHUB_REPO=erikwang2013/bee-rust-admin
[ -z "$BASE_URL" ] && BASE_URL="https://github.com/$GITHUB_REPO/releases/download"

# ══════════════════════════════════════════════════════════════════════════
#  小工具
# ══════════════════════════════════════════════════════════════════════════
# sed 替换串里 & | \ 都是元字符，必须转义（路径里出现 & 是合法的）
esc_repl() { printf '%s' "$1" | sed -e 's/[\\&|]/\\&/g'; }

# DSN 走 Url::parse，口令里的 / # ? % & + 会破解析 → 百分号编码
urlenc() {
  printf '%s' "$1" | sed -e 's/%/%25/g' -e 's/@/%40/g' -e 's/:/%3A/g' \
    -e 's|/|%2F|g' -e 's/#/%23/g' -e 's/?/%3F/g' -e 's/&/%26/g' -e 's/+/%2B/g'
}
# urlenc 的逆（--keep-conf 时把已有 conf 里的 DSN 读回来）。%25 必须**最后**换：
# 换早了，字面量 "%3A"（编码后是 %253A）会被误解成冒号。
urldec() {
  printf '%s' "$1" | sed -e 's/%3A/:/g' -e 's|%2F|/|g' -e 's/%23/#/g' \
    -e 's/%3F/?/g' -e 's/%40/@/g' -e 's/%26/\&/g' -e 's/%2B/+/g' -e 's/%25/%/g'
}

# 随机口令。**不要写成 `tr < /dev/urandom | head -c N`**：head 提前退出会让 tr 吃
# SIGPIPE，配合 `set -o pipefail` 就是随机中止整个脚本。这里让 tr 读**有限**输入
# （head -c 128 自己会结束），循环兜住取不够的极端情况。
gen_pass() {
  local n=${1:-24} set=${2:-'A-Za-z0-9'} s=''
  while [ "${#s}" -lt "$n" ]; do
    s="$s$(LC_ALL=C head -c 128 /dev/urandom | LC_ALL=C tr -dc "$set")"
  done
  printf '%s' "${s:0:$n}"
}
gen_pass_h() { gen_pass "$1" 'A-HJ-NP-Za-km-z2-9'; }

# 取 INI 值。只跳过整行注释（不剥行内 #：值里可能有 #）
ini_get() {
  awk -v sec="$2" -v key="$3" '
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*\[/ { s = $0; gsub(/[][]/, "", s); gsub(/[[:space:]]/, "", s); next }
    s == sec {
      p = index($0, "="); if (p == 0) next
      k = substr($0, 1, p - 1); gsub(/^[[:space:]]+|[[:space:]]+$/, "", k)
      if (k == key) { v = substr($0, p + 1); gsub(/^[[:space:]]+|[[:space:]]+$/, "", v); print v; exit }
    }' "$1"
}

# 密钥指纹：用于「认出这是哪把密钥」但**不泄露密钥本身**
key_fingerprint() { printf '%s' "$1" | sha256sum | cut -c1-12; }

have() { command -v "$1" >/dev/null 2>&1; }

# ⚠️ 本脚本开了 `set -o pipefail`，而 `... | grep -q` 这类**提前退出**的消费者会让上游
# 吃 SIGPIPE(141)，整条管道被判失败 —— 于是「端口被占用」会被读成「空闲」、
# 「缺动态库」会被读成「不缺」、`ldd --version | head -1` 会把 `||` 两边都执行。
# 凡是要用退出码/是否存在判定的一律先取全量再判，不要用 grep -q 收尾管道。
port_busy() {
  ss -ltn 2>/dev/null | awk -v p="$1" '$4 ~ (":" p "$") { f = 1 } END { exit !f }'
}

can_root() { [ "$(id -u)" = 0 ] || { have sudo && sudo -n true 2>/dev/null; }; }
run_root() { # 拿到 root 就跑，否则返回 1（**绝不挂起等口令**）
  if [ "$(id -u)" = 0 ]; then "$@"
  elif have sudo && sudo -n true 2>/dev/null; then sudo -n "$@"
  else return 1; fi
}

# ══════════════════════════════════════════════════════════════════════════
#  模板（与 admin/deploy/*.example 同源；改了那边记得同步这里）
# ══════════════════════════════════════════════════════════════════════════
render_unit() {
  cat <<'EOF'
[Unit]
Description=bee-rust admin backend (@APP_NAME@)
After=network.target mysql.service
Wants=mysql.service

[Service]
Type=simple
User=@RUN_USER@
Group=@RUN_GROUP@
WorkingDirectory=@PREFIX@
Environment=BEE_ADMIN_CONF=@PREFIX@/conf/app.conf
ExecStart=@PREFIX@/bin/bee_admin
Restart=on-failure
RestartSec=3
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF
}

# ⚠️ gzip 那几行不是可选项：首屏 JS 未压缩 ~1.1MB，gzip 后 ~364KB，而 nginx 默认
#    只压 text/html —— 不显式列出 js/css，压缩等于没开。
render_vhost() {
  cat <<'EOF'
# BRA 管理后台 —— nginx 站点（由 install.sh 渲染，改这里要同步 admin/deploy/nginx.conf.example）
server {
    listen @HTTP_PORT@;
    server_name _;

    root @PREFIX@/web/dist;
    index index.html;

    gzip on;
    gzip_types text/css text/javascript application/javascript application/json image/svg+xml;
    gzip_min_length 1024;
    gzip_vary on;

    location / {
        try_files $uri $uri/ /index.html;
    }

    location /api/ {
        # 不带尾斜杠：原样转发完整 URI（/api/v1/auth/login 保持 /api/v1/auth/login）
        proxy_pass http://127.0.0.1:@BACKEND_PORT@;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location ~* \.(js|css|woff2?|svg|png|jpg|ico)$ {
        expires 30d;
        add_header Cache-Control "public, immutable";
    }
    location = /index.html {
        add_header Cache-Control "no-cache";
    }
}
EOF
}

# app.conf 模板（与 admin/conf/app.conf.example 同源）
render_conf() {
  cat <<EOF
# 由 install.sh 生成于 $(date -u '+%Y-%m-%d %H:%M:%S UTC')
#
# ⚠️ encrypt_key 与 hashids_salt **不可更换**：
#    换 encrypt_key = 库里已加密的 email/phone 全部解不开（没有救援）；
#    换 hashids_salt = 所有已发出的对外短串作废。改前先备份这个文件。
[app]
name = $APP_NAME
http_addr = 127.0.0.1:$BACKEND_PORT
log_level = info
upload_dir = uploads
hashids_salt = $HASHIDS_SALT
hashids_min_len = 8
snowflake_worker = ${SNOWFLAKE%,*}
snowflake_dc = ${SNOWFLAKE#*,}
encrypt_key = $ENCRYPT_KEY

[db]
dsn = mysql://$(urlenc "$DB_USER"):$(urlenc "$DB_PASS")@$DB_HOST:$DB_PORT/$DB_NAME

[jwt]
secret = $JWT_SECRET
expire_hours = 24

[auth]
max_fail = 5
lock_minutes = 10
captcha = true

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
initial_admin_password = $ADMIN_PASSWORD
EOF
}

# ══════════════════════════════════════════════════════════════════════════
#  0. 干跑：把将要写的一切打出来就结束
# ══════════════════════════════════════════════════════════════════════════
if [ "$DRY_RUN" = 1 ]; then
  say "干跑（--dry-run）：以下内容**不会**落盘"
  info "模式：$MODE   版本：$VERSION   安装根：$PREFIX"
  info "服务名：$SERVICE_NAME   对外端口：$HTTP_PORT   后端端口：$BACKEND_PORT"
  info "数据库：$DB_USER@$DB_HOST:$DB_PORT/$DB_NAME"
  if [ "$MODE" = download ]; then
    info "产物地址：$BASE_URL/$VERSION/bee_admin（以及 bee-admin-web-dist.tar.gz）"
  else
    info "源码目录：$REPO_DIR（cargo build --release + pnpm build）"
  fi
  printf '\n%s──── conf/app.conf（将要写入）────%s\n' "$C_BLD" "$C_OFF" >&2
  [ -n "$ENCRYPT_KEY" ] || ENCRYPT_KEY="base64:<随机生成的 32 字节>"
  [ -n "$HASHIDS_SALT" ] || HASHIDS_SALT="<随机 32 位 hex>"
  [ -n "$JWT_SECRET" ] || JWT_SECRET="<随机 64 位 hex>"
  [ -n "$DB_PASS" ] || DB_PASS="<随机 24 位字母数字>"
  [ -n "$ADMIN_PASSWORD" ] || ADMIN_PASSWORD="<随机 16 位>"
  render_conf >&2
  printf '\n%s──── systemd 单元 %s.service ────%s\n' "$C_BLD" "$SERVICE_NAME" "$C_OFF" >&2
  render_unit | sed -e "s|@APP_NAME@|$(esc_repl "$APP_NAME")|g" \
    -e "s|@RUN_USER@|$(esc_repl "${RUN_USER:-$(id -un)}")|g" \
    -e "s|@RUN_GROUP@|$(esc_repl "${RUN_GROUP:-$(id -gn)}")|g" \
    -e "s|@PREFIX@|$(esc_repl "$PREFIX")|g" >&2
  printf '\n%s──── nginx 站点 %s ────%s\n' "$C_BLD" "$SERVICE_NAME" "$C_OFF" >&2
  render_vhost | sed -e "s|@HTTP_PORT@|$(esc_repl "$HTTP_PORT")|g" \
    -e "s|@BACKEND_PORT@|$(esc_repl "$BACKEND_PORT")|g" \
    -e "s|@PREFIX@|$(esc_repl "$PREFIX")|g" >&2
  printf '\n%s干跑结束，没有写任何文件。%s\n' "$C_GRN" "$C_OFF" >&2
  exit "$EXIT_OK"
fi

# ══════════════════════════════════════════════════════════════════════════
#  1. 预检（只读；任何一步不过都不写盘）
# ══════════════════════════════════════════════════════════════════════════
say "预检"

arch=$(uname -s)-$(uname -m)
[ "$(uname -s)" = Linux ] || step_fail "$EXIT_PREFLIGHT" "只支持 Linux（当前 $arch）"
[ "$(uname -m)" = x86_64 ] || step_fail "$EXIT_PREFLIGHT" \
  "只提供 x86_64 产物（当前 $arch）。ARM 请用 --mode build 从源码构建，或用 Docker"

missing=''
for c in curl tar openssl install sed awk grep ss sha256sum; do
  have "$c" || missing="$missing $c"
done
[ -z "$missing" ] || step_fail "$EXIT_PREFLIGHT" "缺少命令：$missing（装完再跑）"

if [ "$MODE" = build ]; then
  have cargo || step_fail "$EXIT_PREFLIGHT" "--mode build 需要 cargo（rust 1.99+）"
  have node  || step_fail "$EXIT_PREFLIGHT" "--mode build 需要 node（20+）"
  have pnpm  || step_fail "$EXIT_PREFLIGHT" "--mode build 需要 pnpm（9+）"
  [ -f "$REPO_DIR/Cargo.toml" ] || step_fail "$EXIT_PREFLIGHT" \
    "--repo 指的目录里没有 Cargo.toml：$REPO_DIR"
fi

# --keep-conf 是升级路径：下面第 2 步要靠已有 conf 里的 dsn 决定连哪个库，
# 没有 conf 就没得「保留」——在写盘之前拦住。
[ "$KEEP_CONF" = 0 ] || [ -f "$CONF" ] || step_fail "$EXIT_PREFLIGHT" \
  "--keep-conf 但 $CONF 不存在。它是升级用的（先正常装一次，再用它换二进制）；全新安装去掉这个参数"

if [ -e "$PREFIX" ] && [ ! -w "$PREFIX" ] && ! can_root; then
  step_fail "$EXIT_PREFLIGHT" "$PREFIX 不可写，且没有免密 sudo。
  换个地方：--prefix ~/bee-admin    或用 root 跑：sudo bash $PROG"
fi

for p in "$HTTP_PORT" "$BACKEND_PORT"; do
  if port_busy "$p"; then
    step_fail "$EXIT_PREFLIGHT" "端口 $p 已被占用。
  换端口：--http-port 8091 --backend-port 8090（或先停掉占用它的进程）"
  fi
done

if [ -n "$RUN_USER" ] && ! id -u "$RUN_USER" >/dev/null 2>&1; then
  step_fail "$EXIT_PREFLIGHT" "用户不存在：$RUN_USER"
fi
[ -n "${RUN_USER:-}" ] || RUN_USER=$( [ "$(id -u)" = 0 ] && echo "${SUDO_USER:-root}" || id -un )
[ -n "${RUN_GROUP:-}" ] || RUN_GROUP=$(id -gn "$RUN_USER" 2>/dev/null || id -gn)

if [ "$HTTP_PORT" -lt 1024 ] && ! can_root; then
  warn "端口 $HTTP_PORT < 1024，非 root 起不来 nginx；建议换个高位端口"
fi
info "架构 $arch · 用户 $RUN_USER:$RUN_GROUP · 端口 $HTTP_PORT → $BACKEND_PORT"

# ══════════════════════════════════════════════════════════════════════════
#  2. 已有配置的保护（在任何写入之前判定）
# ══════════════════════════════════════════════════════════════════════════
OLD_CONF_BACKUP=''
if [ -f "$CONF" ]; then
  old_key=$(ini_get "$CONF" app encrypt_key)
  old_fp=$(key_fingerprint "$old_key")
  if [ "$KEEP_CONF" = 1 ]; then
    say "复用已有配置（--keep-conf）"
    info "$CONF 保持原样不动（encrypt_key 指纹 $old_fp）"
    conf_addr=$(ini_get "$CONF" app http_addr)
    conf_upload=$(ini_get "$CONF" app upload_dir)
    # 只报告不修：配置是事实来源
    case ${conf_addr##*:} in
      "$BACKEND_PORT") ;;
      '') warn "已有配置里没有 [app] http_addr —— 单元渲染按 --backend-port $BACKEND_PORT 走" ;;
      *) warn "已有配置里 http_addr 是 $conf_addr，与 --backend-port $BACKEND_PORT 不一致；以配置为准，nginx 会反代到 ${conf_addr##*:}" ;;
    esac
    [ -n "$conf_upload" ] || warn "已有配置里没有 [app] upload_dir（默认 uploads）"
    case $old_key in
      ''|*change-me*) warn "已有配置的 encrypt_key 是空的/占位符 —— 服务**起不来**，请先修好再装" ;;
    esac
    conf_jwt=$(ini_get "$CONF" jwt secret)
    [ ${#conf_jwt} -ge 32 ] || warn "已有配置的 [jwt] secret 不足 32 字符 —— 服务起不来"
    # 库/账号也得按已有 DSN 走：保留的 conf 就是这个服务的连接参数，命令行上现生成的
    # 随机口令在这里毫无意义。**曾经没这么干**：拿新随机 DB_PASS 去连 → 连不上 →
    # 走建库建号 → ALTER USER 把库里的账号口令改掉，而 conf 里还是旧口令 ——
    # 所谓「升级」一次就把自己装坏，自检必然 Access denied。
    conf_dsn=$(ini_get "$CONF" db dsn)
    if [ -z "$conf_dsn" ]; then
      warn "已有配置里没有 [db] dsn —— 数据库检查退回命令行参数"
    elif [ "${conf_dsn#mysql://}" = "$conf_dsn" ]; then
      warn "已有配置里的 [db] dsn 不是 mysql:// 形式 —— 数据库检查退回命令行参数"
    else
      rest=${conf_dsn#mysql://}; userpass=${rest%@*}; hostpart=${rest#*@}
      DB_USER=$(urldec "${userpass%%:*}"); DB_PASS=$(urldec "${userpass#*:}")
      # 先切掉 /db 再切 host:port：`${hostpart##*:}` 会一路吃到路径里的冒号位置，
      # 把端口读成 "3306/bee_admin_install_test"（自检时逮到的）
      DB_NAME=${hostpart#*/}; DB_NAME=${DB_NAME%%\?*}
      hostport=${hostpart%%/*}
      DB_HOST=${hostport%%:*}; DB_PORT=${hostport##*:}
      case $hostport in *:*) ;; *) DB_PORT=3306 ;; esac
      info "数据库按已有配置：$DB_USER@$DB_HOST:$DB_PORT/$DB_NAME（--keep-conf 下 --db-* 不参与）"
    fi
  elif [ "$FORCE" = 1 ]; then
    backup="$CONF.bak.$(date -u '+%Y%m%d%H%M%S')"
    say "覆盖已有配置（--force）"
    cp -p "$CONF" "$backup"      # cp -p 保留权限/时间；**里面含原密钥，所以可恢复**
    OLD_CONF_BACKUP=$backup
    warn "已覆盖 $CONF，原文件备份到 $backup"
    warn "新生成的 encrypt_key 与旧的**不同**：库里已加密的 email/phone 将解不开；"
    warn "hashids_salt 也变了：已发出的对外短串全部作废。要回滚就用上面那个备份。"
  else
    printf '\n%s[中止]%s 已经装过（%s 存在），没有做任何修改。\n\n' "$C_YEL" "$C_OFF" "$CONF" >&2
    info "配置文件：$CONF"
    info "属主/权限：$(stat -c '%U:%G %a' "$CONF" 2>/dev/null || echo '?')   修改时间：$(stat -c '%y' "$CONF" 2>/dev/null | cut -d. -f1)"
    info "encrypt_key 指纹：$old_fp（只用于认出是哪把密钥，不是密钥本身）"
    cat >&2 <<EOF

  三选一：
    1) 升级（推荐）：只换二进制与服务文件，配置一个字节不动
         bash $PROG --keep-conf$(printf ' %s' "${ORIG_ARGS[@]+"${ORIG_ARGS[@]}"}")
    2) 重装配置（危险）：先备份再覆盖 —— encrypt_key 变了库里 email/phone 解不开
         bash $PROG --force ...
    3) 什么都不做（就是现在这样）

EOF
    exit "$EXIT_CONF_EXISTS"
  fi
fi

# ══════════════════════════════════════════════════════════════════════════
#  3. 生成密钥 / 交互补齐（只在这之后才开始写盘）
# ══════════════════════════════════════════════════════════════════════════
[ -n "$ENCRYPT_KEY" ]   || ENCRYPT_KEY="base64:$(openssl rand -base64 32 | tr -d '\n')"
[ -n "$HASHIDS_SALT" ]  || HASHIDS_SALT=$(openssl rand -hex 16)
[ -n "$JWT_SECRET" ]    || JWT_SECRET=$(openssl rand -hex 32)
DB_PASS_GENERATED=0
ADMIN_PASS_GENERATED=0
[ -n "$DB_PASS" ]       || { DB_PASS=$(gen_pass 24);     DB_PASS_GENERATED=1; }
[ -n "$ADMIN_PASSWORD" ]|| { ADMIN_PASSWORD=$(gen_pass_h 16); ADMIN_PASS_GENERATED=1; }

# 交互式补 DB 口令：只在「没给口令（刚才是随机的）」且「没 --yes」时问。
# 从 /dev/tty 读，所以 `curl | bash` 也问得出来（那时 stdin 是脚本本身）。
ask_db_pass=0
if [ "$KEEP_CONF" = 0 ] && [ "$DB_PASS_GENERATED" = 1 ] && [ "$ASSUME_YES" = 0 ] && [ -r /dev/tty ]; then
  ask_db_pass=1
fi
if [ "$ask_db_pass" = 1 ]; then
  printf '\n数据库 %s@%s:%s/%s 的口令（直接回车=用随机生成的）：' "$DB_USER" "$DB_HOST" "$DB_PORT" "$DB_NAME" >&2
  read -rs _p </dev/tty || _p=''
  printf '\n' >&2
  if [ -n "${_p:-}" ]; then DB_PASS=$_p; DB_PASS_GENERATED=0; fi
  unset _p
fi

if [ "$KEEP_CONF" = 0 ]; then
  # 注意：`&` 不能放进 [ ] 里（bash 5.2 解析 case 模式时会报语法错误），用 | 分支
  case $DB_PASS in *[/#?%]*|*"&"*|*"+"*) warn "数据库口令里含 / # ? % & + —— 已做 URL 编码写进 DSN，但别的工具里也可能咬人" ;; esac
fi

# ══════════════════════════════════════════════════════════════════════════
#  4. 取产物
# ══════════════════════════════════════════════════════════════════════════
say "准备产物（$MODE）"
TMP=$PREFIX/tmp
# 注意与下面下载/构建落到的是**同一个**路径（曾经这里写成 bee_admin.new，而下载写的是
# bee_admin，于是二进制永远装不上——自检把它抓出来了）
BIN_NEW=$TMP/bee_admin
DIST_NEW=$TMP/web-dist.new

if [ "$MODE" = download ]; then
  [ "$VERSION" = latest ] && VERSION=$(curl -fsSL --max-time 20 \
    "https://api.github.com/repos/$GITHUB_REPO/releases/latest" 2>/dev/null \
    | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | cut -d'"' -f4)
  [ -n "${VERSION:-}" ] && [ "$VERSION" != latest ] || step_fail "$EXIT_PREFLIGHT" \
    "拿不到最新版本号（GitHub 不通？）。显式指定：--version v2.6.0 或用 --base-url 指内网镜像"
  info "版本：$VERSION"

  if [ "$(cat "$VERSION_FILE" 2>/dev/null || true)" = "$VERSION" ] && [ -x "$PREFIX/bin/bee_admin" ]; then
    say "已经是 $VERSION，跳过下载（要强制重装就删掉 $VERSION_FILE）"
  else
    mkdir -p "$TMP"
    for f in bee_admin bee-admin-web-dist.tar.gz; do
      src="$BASE_URL/$VERSION/$f"
      info "拉取 $src"
      ok=0
      for try in 1 2 3; do
        case $src in
          file://*) cp -- "${src#file://}" "$TMP/$f" 2>/dev/null && ok=1 ;;
          *) curl -fL --retry 3 --retry-delay 2 --connect-timeout 15 --max-time 600 \
               -o "$TMP/$f" "$src" 2>/dev/null && ok=1 ;;
        esac
        [ "$ok" = 1 ] && break
        [ "$try" -lt 3 ] && { warn "第 $try 次下载失败，重试…"; sleep 3; }
      done
      [ "$ok" = 1 ] || step_fail "$EXIT_PREFLIGHT" \
        "下载失败：$src
  版本不存在？网络不通？内网就指定镜像：--base-url http://你的镜像/releases/download"
    done
    # **curl/cp 都不会给下载来的文件加执行位**，而下面第 5 步要真的运行它来判定可运行性。
    # （T4 之所以没暴露这条：那次用的是本地 `install -m0755` 造出来的产物，本来就带 +x。）
    chmod +x "$TMP/bee_admin"
    # 404 会返回一段 HTML，别把它当二进制装上
    magic=$(head -c4 "$TMP/bee_admin" | od -An -tx1 | tr -d ' \n')
    [ "$magic" = "7f454c46" ] || step_fail "$EXIT_PREFLIGHT" \
      "$TMP/bee_admin 不是 ELF 可执行文件（下到的可能是 404 页面；检查 --version 与 --base-url）"
    info "sha256：$(sha256sum "$TMP/bee_admin" | cut -d' ' -f1)"
    info "sha256：$(sha256sum "$TMP/bee-admin-web-dist.tar.gz" | cut -d' ' -f1)"
  fi
else
  [ -z "$REPO_DIR" ] && step_fail "$EXIT_PREFLIGHT" "--mode build 需要 --repo 指到源码目录"
  mkdir -p "$TMP"
  say "从源码构建（这一步慢，几分钟到几十分钟）"
  ( cd "$REPO_DIR" && cargo build --release -p bee_admin ) || step_fail "$EXIT_FAIL" "cargo 构建失败"
  ( cd "$REPO_DIR/admin/web" && pnpm install --frozen-lockfile && pnpm build ) \
    || step_fail "$EXIT_FAIL" "前端构建失败"
  install -m0755 "$REPO_DIR/target/release/bee_admin" "$BIN_NEW"
  tar czf "$TMP/bee-admin-web-dist.tar.gz" -C "$REPO_DIR/admin/web/dist" .
  VERSION_FILE_VALUE="built-$(date -u '+%Y%m%d%H%M%S')"
fi

# ══════════════════════════════════════════════════════════════════════════
#  5. 产物可运行性检查（download 模式；**写盘之前**）
# ══════════════════════════════════════════════════════════════════════════
if [ "$MODE" = download ] && [ -f "$BIN_NEW" ]; then
  ldd_out=$(ldd "$BIN_NEW" 2>&1 || true)
  case $ldd_out in *'not found'*)
    step_fail "$EXIT_PREFLIGHT" "$(cat <<EOF
预编译产物缺动态库（ldd 报 not found）：
$(printf '%s\n' "$ldd_out" | grep 'not found' | sed 's/^/    /')

  改用源码构建：  bash $PROG --mode build
  或用 Docker：   cp .env.example .env && cp admin/conf/app.conf.example admin/conf/app.conf \\
                  && vi .env admin/conf/app.conf && docker compose up -d --build
EOF
)"
    ;; esac
  # 让产物真的跑一次。**用「正面特征」判定，而不是查报错文字**：加载失败的花样太多
  # （GLIBC 版本不够、缺 libgomp、架构不对、文件被截断），实测一个截断的 ELF 会直接
  # 以 135 挂掉、一个字都不打，光靠 grep 报错会漏。能跑起来的话它一定走到自己的配置层，
  # 打出 `Error: Missing { section: … }`（main 返回 Result，运行时这么打印）。
  probe_out=''
  probe_out=$(BEE_ADMIN_CONF=/dev/null "$BIN_NEW" 2>&1) || true
  case $probe_out in
    *"Error: Missing"*)
      info "产物可运行性检查通过（能加载、能走到配置层）"
      ;;
    *)
      host_glibc=$(ldd --version 2>/dev/null | awk 'NR==1{ for (i=NF;i>0;i--) if ($i ~ /^[0-9]+\.[0-9]+$/) { print $i; exit } }')
      host_glibc=${host_glibc:-未知}
      step_fail "$EXIT_PREFLIGHT" "$(cat <<EOF
预编译产物在本机跑不起来。它只说了这些：
$(printf '%s\n' "${probe_out:-<什么都没输出，进程直接挂了>}" | sed -n '1,4p' | sed 's/^/    /')

  本机 glibc：$host_glibc —— 预编译产物需要 **2.38**（Debian 13 / Ubuntu 24.04+）。
  两条路（都不动磁盘，随便挑）：
    1) 从源码构建：  bash $PROG --mode build
    2) 用 Docker：   cp .env.example .env && cp admin/conf/app.conf.example admin/conf/app.conf \\
                     && vi .env admin/conf/app.conf && docker compose up -d --build
EOF
)"
      ;;
  esac
fi

# ══════════════════════════════════════════════════════════════════════════
#  6. 落产物（先写临时名再原子换位，绝不往线上目录里解压）
# ══════════════════════════════════════════════════════════════════════════
say "安装到 $PREFIX"
mkdir -p "$PREFIX/bin" "$PREFIX/conf" "$PREFIX/web" "$PREFIX/uploads" "$STAGE_ETC" "$TMP"

if [ -f "$BIN_NEW" ]; then
  install -m0755 "$BIN_NEW" "$TMP/bee_admin.staged"
  [ -f "$PREFIX/bin/bee_admin" ] && mv -f "$PREFIX/bin/bee_admin" "$PREFIX/bin/bee_admin.prev"
  mv -f "$TMP/bee_admin.staged" "$PREFIX/bin/bee_admin"
  info "二进制 → $PREFIX/bin/bee_admin$([ -f "$PREFIX/bin/bee_admin.prev" ] && echo "（旧版留作 bee_admin.prev，回滚就一条 mv）")"
fi

if [ -f "$TMP/bee-admin-web-dist.tar.gz" ]; then
  rm -rf "$DIST_NEW"; mkdir -p "$DIST_NEW"
  tar xzf "$TMP/bee-admin-web-dist.tar.gz" -C "$DIST_NEW"
  [ -f "$DIST_NEW/index.html" ] || step_fail "$EXIT_FAIL" "前端产物里没有 index.html（压缩包内容不对）"
  rm -rf "$PREFIX/web/dist.old"
  [ -d "$PREFIX/web/dist" ] && mv "$PREFIX/web/dist" "$PREFIX/web/dist.old"
  mv "$DIST_NEW" "$PREFIX/web/dist"
  rm -rf "$PREFIX/web/dist.old"
  chmod -R a+rX "$PREFIX/web/dist"     # nginx 的 worker 用户（www/www-data）要能读
  info "前端 → $PREFIX/web/dist"
fi

if [ "$KEEP_CONF" = 0 ]; then
  render_conf > "$TMP/app.conf.new"
  install -m0600 "$TMP/app.conf.new" "$CONF"
  info "配置 → $CONF（0600）"
fi
printf '%s\n' "${VERSION_FILE_VALUE:-$VERSION}" > "$VERSION_FILE"

# ══════════════════════════════════════════════════════════════════════════
#  7. 数据库
# ══════════════════════════════════════════════════════════════════════════
say "检查数据库"
db_ready=0
if have mysql; then
  # 先试应用账号（库/号已存在的老机器走这条）
  if MYSQL_PWD=$DB_PASS mysql -h"$DB_HOST" -P"$DB_PORT" -u"$DB_USER" -D"$DB_NAME" \
       -e 'SELECT 1' >/dev/null 2>&1; then
    info "应用账号可连（$DB_USER@$DB_HOST:$DB_PORT/$DB_NAME）"
    db_ready=1
  elif [ -n "$DB_ADMIN_USER" ] && [ "$KEEP_CONF" = 1 ]; then
    # 保留的配置里是旧口令，ALTER USER 只会把库和配置改成对不上
    warn "--keep-conf：不建库建号（配置里的 DSN 是事实来源）。先按上面的提示修好 dsn 再重跑"
  elif [ -n "$DB_ADMIN_USER" ]; then
    say "用管理账号建库建号"
    grant_host=$DB_HOST
    case $grant_host in 127.0.0.1|localhost) grant_host=127.0.0.1 ;; esac
    sql=$(cat <<EOF
CREATE DATABASE IF NOT EXISTS \`$DB_NAME\` CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;
CREATE USER IF NOT EXISTS '$DB_USER'@'$grant_host' IDENTIFIED BY '$(printf '%s' "$DB_PASS" | sed "s/'/''/g")';
ALTER USER '$DB_USER'@'$grant_host' IDENTIFIED BY '$(printf '%s' "$DB_PASS" | sed "s/'/''/g")';
GRANT ALL PRIVILEGES ON \`$DB_NAME\`.* TO '$DB_USER'@'$grant_host';
FLUSH PRIVILEGES;
EOF
)
    info "将执行："
    printf '%s\n' "$sql" | sed 's/^/    /' >&2
    if printf '%s\n' "$sql" | MYSQL_PWD=$DB_ADMIN_PASS mysql -h"$DB_HOST" -P"$DB_PORT" \
         -u"$DB_ADMIN_USER" >/dev/null 2>&1; then
      if MYSQL_PWD=$DB_PASS mysql -h"$DB_HOST" -P"$DB_PORT" -u"$DB_USER" -D"$DB_NAME" \
           -e 'SELECT 1' >/dev/null 2>&1; then
        info "建好了，应用账号连得上"
        db_ready=1
      else
        warn "建库执行了，但应用账号仍连不上（授权主机名？口令？）"
      fi
    else
      warn "管理账号执行失败（权限不足或口令不对）"
    fi
  fi
else
  warn "本机没有 mysql 客户端，跳过连接测试"
fi

if [ "$db_ready" = 0 ]; then
  cat >&2 <<EOF

${C_YEL}需要你先建好数据库${C_OFF}（脚本不 DROP、不猜 root 口令、不装 MySQL）。
用有 CREATE USER 权限的账号执行（口令换成你自己的）：

  CREATE DATABASE IF NOT EXISTS \`$DB_NAME\` CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;
  CREATE USER IF NOT EXISTS '$DB_USER'@'127.0.0.1' IDENTIFIED BY '你的口令';
  GRANT ALL PRIVILEGES ON \`$DB_NAME\`.* TO '$DB_USER'@'127.0.0.1';
  FLUSH PRIVILEGES;

建好之后重跑本脚本（或加 --db-admin-user/--db-admin-pass 让它自动建）。
表结构与超管账号由服务**首次启动时自动创建**，不需要手工导入 SQL。

EOF
  step_fail "$EXIT_NEED_DB" "数据库还没就绪"
fi

# ══════════════════════════════════════════════════════════════════════════
#  8. 渲染并安装 systemd 单元 / nginx 站点
# ══════════════════════════════════════════════════════════════════════════
need_root_cmds=''

render_unit | sed -e "s|@APP_NAME@|$(esc_repl "$APP_NAME")|g" \
  -e "s|@RUN_USER@|$(esc_repl "$RUN_USER")|g" \
  -e "s|@RUN_GROUP@|$(esc_repl "$RUN_GROUP")|g" \
  -e "s|@PREFIX@|$(esc_repl "$PREFIX")|g" > "$TMP/unit.rendered"
render_vhost | sed -e "s|@HTTP_PORT@|$(esc_repl "$HTTP_PORT")|g" \
  -e "s|@BACKEND_PORT@|$(esc_repl "$BACKEND_PORT")|g" \
  -e "s|@PREFIX@|$(esc_repl "$PREFIX")|g" > "$TMP/vhost.rendered"

install -m0644 "$TMP/unit.rendered" "$UNIT_FILE"
install -m0644 "$TMP/vhost.rendered" "$VHOST_FILE"
info "已渲染：$UNIT_FILE"
info "已渲染：$VHOST_FILE"

# 后端实际监听地址：--keep-conf 时以配置为准
listen_addr=$(ini_get "$CONF" app http_addr 2>/dev/null || true)
listen_port=${listen_addr##*:}; [ -n "$listen_port" ] || listen_port=$BACKEND_PORT

if [ "$WITH_SYSTEMD" = 1 ]; then
  say "安装 systemd 单元"
  if run_root install -m0644 "$UNIT_FILE" "/etc/systemd/system/$SERVICE_NAME.service"; then
    run_root systemctl daemon-reload || true
    run_root systemctl enable "$SERVICE_NAME" >/dev/null 2>&1 || true
    info "已安装 /etc/systemd/system/$SERVICE_NAME.service"
  else
    need_root_cmds="$need_root_cmds
  sudo install -m0644 '$UNIT_FILE' '/etc/systemd/system/$SERVICE_NAME.service'
  sudo systemctl daemon-reload && sudo systemctl enable --now $SERVICE_NAME"
    warn "没有 root（或免密 sudo 不可用），systemd 单元只渲染到 $UNIT_FILE"
  fi
fi

if [ "$WITH_NGINX" = 1 ]; then
  say "安装 nginx 站点"
  ndir=$NGINX_CONF_DIR
  if [ -z "$ndir" ] && have nginx; then
    # 自编译版（LNMP）的站点目录在 --prefix 下，且**只有被 nginx.conf include 的才算**；
    # 发行版在 /etc/nginx 下。都不认就宁可只暂存 + 打印命令，也不猜着写。
    npfx=$(nginx -V 2>&1 | tr ' ' '\n' | sed -n 's/^--prefix=//p' | tail -1)
    if [ -n "$npfx" ] && [ -f "$npfx/conf/nginx.conf" ]; then
      for sub in conf/vhost conf/conf.d; do
        cand="$npfx/$sub"
        if [ -d "$cand" ] && grep -qE "include[[:space:]]+.*$(basename "$cand")" "$npfx/conf/nginx.conf" 2>/dev/null; then
          ndir=$cand; break
        fi
      done
    fi
    if [ -z "$ndir" ] && [ -d /etc/nginx/conf.d ] \
       && grep -qE 'include[[:space:]]+.*conf\.d' /etc/nginx/nginx.conf 2>/dev/null; then
      ndir=/etc/nginx/conf.d
    fi
    if [ -z "$ndir" ] && [ -d /etc/nginx/sites-enabled ]; then ndir=/etc/nginx/sites-enabled; fi
  fi
  target="${ndir:-}/$SERVICE_NAME.conf"
  if [ -n "$ndir" ] && [ -e "$target" ] && ! cmp -s "$VHOST_FILE" "$target" && [ "$FORCE" != 1 ]; then
    warn "$target 已存在且内容不同 —— 没有覆盖（加 --force 才覆盖，或删掉那个文件）"
    ndir=''
  fi
  if [ -n "$ndir" ] && run_root install -m0644 "$VHOST_FILE" "$target"; then
    if run_root nginx -t >/dev/null 2>&1; then
      if have systemctl && systemctl is-active nginx >/dev/null 2>&1; then
        run_root systemctl reload nginx || warn "reload nginx 失败，看 nginx -t 的输出"
      else
        run_root nginx -s reload || warn "nginx -s reload 失败（自编译版可能没有 systemd 单元）"
      fi
      info "已安装并 reload：$target"
    else
      run_root rm -f "$target"
      warn "nginx -t 不通过，已把 $target 撤掉。检查：nginx -t"
    fi
  else
    need_root_cmds="$need_root_cmds
  sudo install -m0644 '$VHOST_FILE' '${ndir:-/etc/nginx/conf.d}/$SERVICE_NAME.conf'
  sudo nginx -t && sudo systemctl reload nginx"
    warn "没有 root（或没探到 nginx 站点目录），vhost 只渲染到 $VHOST_FILE"
  fi
fi

# ══════════════════════════════════════════════════════════════════════════
#  9. 启动 + 自检
# ══════════════════════════════════════════════════════════════════════════
manual_start=0
if [ "$WITH_SYSTEMD" != 1 ]; then
  # --no-systemd：单元文件根本没装进系统，`restart $SERVICE_NAME` 只会打到**同名的别的单元**
  # （默认服务名就是 bee-admin —— 那多半是生产那个），所以这里坚决不碰 systemd。
  # 手动启动命令在收尾处打印（与 need-root 的提示同一口径）。
  say "跳过重启服务（--no-systemd，单元没有装）"
  manual_start=1
elif run_root systemctl restart "$SERVICE_NAME" 2>/dev/null; then
  info "已重启 $SERVICE_NAME.service"
  sleep 2
else
  manual_start=1
fi

selfcheck_http() { # $1=url $2=期望内容
  local i body
  for i in $(seq 1 15); do
    body=$(curl -fsS --max-time 5 "$1" 2>/dev/null || true)
    case $body in *"$2"*) return 0 ;; esac
    sleep 1
  done
  return 1
}

say "自检"
backend_ok=0
if [ "$manual_start" = 0 ]; then
  # systemd 托管：直接探后端端口。
  # 注意**不用登录探针**——默认开着验证码，登录必然 400，测不了；
  # 而健康检查能回 OK 就说明配置解析、DSN、建表、种子全都过去了（任一步失败进程会直接退出）。
  if selfcheck_http "http://127.0.0.1:$listen_port/api/v1/health" OK "后端"; then
    backend_ok=1
    info "后端健康检查通过（127.0.0.1:$listen_port）"
  else
    printf '\n%s[自检失败]%s 后端没起来。看这几样：\n' "$C_RED" "$C_OFF" >&2
    printf '    systemctl status %s -l --no-pager\n    journalctl -u %s -n 100 --no-pager\n' "$SERVICE_NAME" "$SERVICE_NAME" >&2
    printf '    cat %s   # 检查 dsn / encrypt_key / jwt secret（**别贴给别人看**）\n' "$CONF" >&2
    exit "$EXIT_SELFTEST"
  fi
else
  # 没托管：前台起一个进程探完就停（--no-systemd，或 systemd 不可用/没 root）
  say "后端未托管，先直接起一个前台进程探一下"
  BEE_ADMIN_CONF=$CONF BEE_ADMIN_HTTP_ADDR="127.0.0.1:$listen_port" \
    "$PREFIX/bin/bee_admin" > "$TMP/backend.log" 2>&1 &
  probe_pid=$!
  if selfcheck_http "http://127.0.0.1:$listen_port/api/v1/health" OK "后端"; then
    backend_ok=1
    info "后端健康检查通过（进程 $probe_pid，探完就停）"
  else
    printf '\n%s[自检失败]%s 后端没起来。它最后说的话：\n' "$C_RED" "$C_OFF" >&2
    tail -20 "$TMP/backend.log" | sed 's/^/    /' >&2
    cat >&2 <<EOF

  排障：
    cat $CONF              # 检查 dsn / encrypt_key / jwt secret（**别贴给别人看**：里面有密钥和口令）
    手动前台起一次看日志：
      BEE_ADMIN_CONF=$CONF $PREFIX/bin/bee_admin
    渲染好的服务文件在 $UNIT_FILE 与 $VHOST_FILE
EOF
  fi
  kill "$probe_pid" 2>/dev/null || true
  wait "$probe_pid" 2>/dev/null || true
  [ "$backend_ok" = 1 ] || exit "$EXIT_SELFTEST"
fi

# ══════════════════════════════════════════════════════════════════════════
#  10. 收尾
# ══════════════════════════════════════════════════════════════════════════
printf '\n%s════════ 装好了 ════════%s\n' "$C_GRN$C_BLD" "$C_OFF" >&2
info "安装根：$PREFIX    版本：$(cat "$VERSION_FILE" 2>/dev/null || echo '?')"
[ "$manual_start" = 1 ] && {
  if [ -n "$need_root_cmds" ]; then
    printf '\n%s服务还没有被 systemd 托管。执行下面这些就完事（需要 root）：%s\n' "$C_YEL" "$C_OFF" >&2
    printf '%s\n' "$need_root_cmds" >&2
    printf '\n' >&2
  else
    # 走到这里 = 服务没被 systemd 托管（--no-systemd、--no-nginx、或没 root）。
    # 只陈述这一件事：站点装没装是另一码事，别一起说。
    printf '\n%s服务没有被 systemd 托管（--no-systemd / --no-nginx / 没 root）：单元只渲染到 %s/etc/，没装进系统。%s\n' \
      "$C_YEL" "$PREFIX" "$C_OFF" >&2
    printf '  要这么起（前台）：\n' >&2
  fi
  printf '    BEE_ADMIN_CONF=%s %s/bin/bee_admin\n' "$CONF" "$PREFIX" >&2
}

# 整条链路（nginx → 后端）：只有在「root 都装上了、nginx 也 reload 过」时才有意义，
# 否则 nginx 还没加载新站点，测了只会吓人
if [ "$backend_ok" = 1 ] && [ "$manual_start" = 0 ] && [ "$WITH_NGINX" = 1 ] && [ -z "$need_root_cmds" ]; then
  if selfcheck_http "http://127.0.0.1:$HTTP_PORT/api/v1/health" OK "链路"; then
    info "链路自检通过（nginx → 后端）"
  else
    warn "nginx 链路没通（http://127.0.0.1:$HTTP_PORT/api/v1/health）—— 看 nginx -t 与后端日志"
  fi
fi

cat >&2 <<EOF

  打开：  http://<这台机器的地址>:$HTTP_PORT
  账号：  admin
EOF
if [ "$ADMIN_PASS_GENERATED" = 1 ]; then
  printf '  初始口令：%s%s%s   ← 只显示这一次，登录后立刻改\n' "$C_BLD" "$ADMIN_PASSWORD" "$C_OFF" >&2
else
  printf '  初始口令：你通过 --admin-password 指定的那个\n' >&2
fi
[ "$DB_PASS_GENERATED" = 1 ] && \
  printf '  数据库口令（已写进 conf）：%s%s%s\n' "$C_YEL" "$DB_PASS" "$C_OFF" >&2
cat >&2 <<EOF

  说明：
    · 初始口令**只在库为空（首次启动）时生效**；库已有 admin 的话它被忽略，请登录后改密
    · ⚠️ $CONF 里的 encrypt_key **不可更换**（换了库里 email/phone 解不开），备份它
    · 升级再来一次：bash $PROG --keep-conf ...（配置一个字节不动）
    · 回滚二进制：mv $PREFIX/bin/bee_admin.prev $PREFIX/bin/bee_admin && systemctl restart $SERVICE_NAME
    · 卸载：systemctl disable --now $SERVICE_NAME，再删 $PREFIX 与上面的服务文件（数据库自己决定要不要删）
EOF
[ -n "$OLD_CONF_BACKUP" ] && printf '    · 旧配置备份：%s\n' "$OLD_CONF_BACKUP" >&2
[ -n "$OLD_CONF_BACKUP" ] && warn "这次覆盖了配置：库里已加密的 email/phone 用新的 encrypt_key 解不开。"
printf '\n' >&2

# 还有 root 才能做的步骤没做（systemd / nginx）→ 退 3，把「装完了」和「还差一步」区分开
if [ -n "$need_root_cmds" ]; then
  printf '%s还差最后一步%s：上面那两条 sudo 命令（跑完就真的完事了）。\n\n' "$C_YEL" "$C_OFF" >&2
  exit "$EXIT_NEED_ROOT"
fi
exit "$EXIT_OK"
