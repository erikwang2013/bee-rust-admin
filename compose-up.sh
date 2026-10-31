#!/usr/bin/env bash
# BRA 单机 compose 起栈：MySQL + 后端 + 前端 nginx。
#
#   ./compose-up.sh                 # 生成缺失的密钥/口令 → 起栈（默认 HTTP :8081）
#   ./compose-up.sh --build         # 从源码重新构建镜像（要 Rust/Node，慢）
#   ./compose-up.sh --tls           # 强制 HTTPS（要求 certs/ 与 SERVER_NAME）
#   ./compose-up.sh --http          # 强制 HTTP（即使 certs/ 齐全）
#   ./compose-up.sh --regen NAME    # 重新生成一个密钥（all = 全部），旧的备份保留
#   ./compose-up.sh --regen all
#   ./compose-up.sh --help
#
# 密钥/口令的落盘位置（全部在宿主机）：
#   secrets/<name>          0600；首次缺失时自动生成，之后永不覆盖
#   secrets/backups/        重新生成时，旧值在这里按时间戳保留
#   .env                    由 secrets 渲染（0600），compose 读取
#   admin/conf/app.conf     由 secrets 渲染（0600），后端读取
#   certs/fullchain.pem     **唯一需要手工配置的**东西：域名证书
#   certs/privkey.pem
#   data/bra/               上传 / 附件 / 导出 / 备份（挂到后端 /home/uploads）
#
# 日志不写容器层：compose 用 json-file 驱动，max-size/max-file 轮转。
set -euo pipefail
umask 077

ROOT=$(cd -- "$(dirname -- "$0")" 2>/dev/null && pwd -P) || exit 1
cd "$ROOT"

SECRETS=secrets
DATA=data/bra
CERTS=certs
ENV_FILE=.env
ENV_EXAMPLE=.env.example
CONF=admin/conf/app.conf
CONF_EXAMPLE=admin/conf/app.conf.example
SECRET_NAMES=(mysql_root_password mysql_password jwt_secret encrypt_key hashids_salt initial_admin_password)

info() { printf '    %s\n' "$*" >&2; }
warn() { printf '警告: %s\n' "$*" >&2; }
die()  { printf '错误: %s\n' "$*" >&2; exit 1; }

usage() {
  sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
}

need_cmd() { command -v "$1" >/dev/null 2>&1 || die "缺少命令：$1"; }
need_cmd openssl
need_cmd docker

# ── 参数 ──────────────────────────────────────────────────────────────────
REGEN=()
FORCE_HTTP=0
FORCE_TLS=0
UP_ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --regen)
      [ $# -ge 2 ] || die "--regen 需要值：all 或 ${SECRET_NAMES[*]}"
      REGEN+=("$2"); shift 2;;
    --http) FORCE_HTTP=1; shift;;
    --tls)  FORCE_TLS=1;  shift;;
    --help|-h) usage; exit 0;;
    --) shift; UP_ARGS+=("$@"); break;;
    *) UP_ARGS+=("$1"); shift;;
  esac
done
[ $FORCE_HTTP -eq 1 ] && [ $FORCE_TLS -eq 1 ] && die "--http 与 --tls 不能同时给"

# ── 目录与权限 ────────────────────────────────────────────────────────────
mkdir -p "$SECRETS" "$SECRETS/backups" "$DATA" "$CERTS"
chmod 700 "$SECRETS" "$SECRETS/backups"
chmod 750 data "$DATA" 2>/dev/null || true

# ── 读旧值（升级兼容）与渲染工具 ─────────────────────────────────────────
[ -f "$ENV_FILE" ] || { cp "$ENV_EXAMPLE" "$ENV_FILE"; info "已从 $ENV_EXAMPLE 创建 $ENV_FILE"; }
[ -f "$CONF" ] || { cp "$CONF_EXAMPLE" "$CONF"; info "已从 $CONF_EXAMPLE 创建 $CONF"; }
chmod 600 "$ENV_FILE" "$CONF"

env_get() {
  sed -n "s/^$1[[:space:]]*=[[:space:]]*//p" "$ENV_FILE" | head -1 || true
}
env_set() { # key value
  if grep -q "^$1[[:space:]]*=" "$ENV_FILE"; then
    sed -i "s#^$1[[:space:]]*=.*#$1=$2#" "$ENV_FILE"
  else
    printf '%s=%s\n' "$1" "$2" >> "$ENV_FILE"
  fi
}
ini_get() {
  sed -n "s/^$1[[:space:]]*=[[:space:]]*//p" "$CONF" | head -1 || true
}
ini_set() { # key value section
  if grep -q "^$1[[:space:]]*=" "$CONF"; then
    sed -i "s#^$1[[:space:]]*=.*#$1 = $2#" "$CONF"
  else
    sed -i "/^\[$3\][[:space:]]*$/a $1 = $2" "$CONF"
  fi
}

placeholder() {
  case "${1:-}" in
    ''|*change-me*|*changeme*|*change_me*|*CHANGE_ME*) return 0;;
    *) return 1;;
  esac
}

secret_path() { printf '%s/%s' "$SECRETS" "$1"; }

generate_secret() { # name → stdout
  case "$1" in
    mysql_root_password) openssl rand -hex 24;;
    mysql_password)      openssl rand -hex 24;;
    jwt_secret)          openssl rand -hex 32;;
    encrypt_key)         printf 'base64:%s\n' "$(openssl rand -base64 32)";;
    hashids_salt)        openssl rand -hex 16;;
    initial_admin_password) printf 'Pw%s\n' "$(openssl rand -hex 12)";;
    *) die "未知密钥名：$1";;
  esac
}

legacy_value() { # name → old value from .env/app.conf
  case "$1" in
    mysql_root_password) env_get MYSQL_ROOT_PASSWORD;;
    jwt_secret)          ini_get secret;;
    encrypt_key)         ini_get encrypt_key;;
    hashids_salt)        ini_get hashids_salt;;
    initial_admin_password) ini_get initial_admin_password;;
    *) return 0;;
  esac
}

ensure_secret() { # name
  local name=$1 file val
  file=$(secret_path "$name")
  if [ -s "$file" ]; then return 0; fi
  val=$(legacy_value "$name" || true)
  if placeholder "$val"; then
    val=$(generate_secret "$name")
    info "已生成 $name → $file"
  else
    info "已从旧配置迁移 $name → $file"
  fi
  printf '%s\n' "$val" > "$file"
  chmod 600 "$file"
}

secret_value() { # name — file must exist
  local f
  f=$(secret_path "$1")
  [ -s "$f" ] || die "缺少 $f（先跑 ./compose-up.sh 生成）"
  tr -d '\r\n' < "$f"
}

# ── 首次生成 / 迁移 ───────────────────────────────────────────────────────
for name in "${SECRET_NAMES[@]}"; do ensure_secret "$name"; done

# ── --regen：备份旧值 → 生成新值 → 渲染 → 退出（不自动重启）──────────────
regen_one() {
  local name=$1 file old backup
  [[ " ${SECRET_NAMES[*]} " == *" $name "* ]] || die "未知密钥名：$name（可用：all ${SECRET_NAMES[*]}）"
  file=$(secret_path "$name")
  if [ -s "$file" ]; then
    backup="$SECRETS/backups/$name.$(date -u +%Y%m%dT%H%M%SZ)"
    cp "$file" "$backup"
    chmod 600 "$backup"
    info "旧 $name 已备份 → $backup"
  fi
  generate_secret "$name" > "$file"
  chmod 600 "$file"
  info "已重新生成 $name"
  case "$name" in
    encrypt_key) warn "encrypt_key 已变：库里已加密的 email/phone 将无法解密；旧值在 secrets/backups/";;
    hashids_salt) warn "hashids_salt 已变：已发出的对外短 id 全部作废；旧值在 secrets/backups/";;
    jwt_secret) warn "jwt_secret 已变：所有在线会话会失效（重新登录即可）";;
    mysql_root_password|mysql_password) warn "MySQL 口令只改了文件；已有数据库里的密码不会自动改。请先按 docs/faq.md 的方式改库，或用新库；旧值在 secrets/backups/";;
    initial_admin_password) warn "初始口令只在 admin 表为空时生效；已有库不会改现有超管密码";;
  esac
}

if [ ${#REGEN[@]} -gt 0 ]; then
  for n in "${REGEN[@]}"; do
    if [ "$n" = all ]; then
      for name in "${SECRET_NAMES[@]}"; do regen_one "$name"; done
    else
      regen_one "$n"
    fi
  done
  # 重新渲染后退出；用户确认数据库口令等事项后再起栈
  for name in "${SECRET_NAMES[@]}"; do ensure_secret "$name"; done
  # 继续走下面的渲染，然后退出
  REGEN_DONE=1
else
  REGEN_DONE=0
fi

# ── 渲染 .env ─────────────────────────────────────────────────────────────
env_set MYSQL_ROOT_PASSWORD "$(secret_value mysql_root_password)"
env_set MYSQL_PASSWORD "$(secret_value mysql_password)"
MYSQL_DATABASE=$(env_get MYSQL_DATABASE); MYSQL_DATABASE=${MYSQL_DATABASE:-bee_admin}
MYSQL_USER=$(env_get MYSQL_USER); MYSQL_USER=${MYSQL_USER:-bra}
[[ $MYSQL_DATABASE =~ ^[A-Za-z0-9_]+$ ]] || die "MYSQL_DATABASE 只能包含字母/数字/下划线"
[[ $MYSQL_USER =~ ^[A-Za-z0-9_]+$ ]] || die "MYSQL_USER 只能包含字母/数字/下划线"
env_set MYSQL_DATABASE "$MYSQL_DATABASE"
env_set MYSQL_USER "$MYSQL_USER"
BRA_VERSION=$(env_get BRA_VERSION); BRA_VERSION=${BRA_VERSION:-latest}
env_set BRA_VERSION "$BRA_VERSION"
APP_UID=$(env_get APP_UID); APP_UID=${APP_UID:-$(id -u)}
APP_GID=$(env_get APP_GID); APP_GID=${APP_GID:-$(id -g)}
[[ $APP_UID =~ ^[0-9]+$ && $APP_GID =~ ^[0-9]+$ ]] || die "APP_UID / APP_GID 必须是数字"
env_set APP_UID "$APP_UID"
env_set APP_GID "$APP_GID"
TZ_VAL=$(env_get TZ); TZ_VAL=${TZ_VAL:-Asia/Shanghai}
env_set TZ "$TZ_VAL"

# ── 渲染 app.conf ─────────────────────────────────────────────────────────
ini_set dsn "mysql://$MYSQL_USER:$(secret_value mysql_password)@mysql:3306/$MYSQL_DATABASE" db
ini_set encrypt_key "$(secret_value encrypt_key)" app
ini_set hashids_salt "$(secret_value hashids_salt)" app
ini_set secret "$(secret_value jwt_secret)" jwt
ini_set initial_admin_password "$(secret_value initial_admin_password)" seed
# 容器里 WORKDIR=/home，uploads 落在挂载的 /home/uploads
ini_set upload_dir "uploads" app
chmod 600 "$ENV_FILE" "$CONF"

# ── data 目录归属 ─────────────────────────────────────────────────────────
if [ "$(id -u)" = 0 ]; then
  chown -R "$APP_UID:$APP_GID" "$DATA"
else
  if [ "$(id -u)" != "$APP_UID" ]; then
    warn "APP_UID=$APP_UID 与当前用户 $(id -u) 不同；请确保 $DATA 对 uid $APP_UID 可写（或用 root 跑一次 chown -R $APP_UID:$APP_GID $DATA）"
  fi
fi
chmod 700 "$DATA"

# ── TLS 选择 ──────────────────────────────────────────────────────────────
SERVER_NAME=$(env_get SERVER_NAME || true)
if [ $FORCE_TLS -eq 1 ]; then
  [ -n "$SERVER_NAME" ] && ! placeholder "$SERVER_NAME" || die "--tls 需要 .env 里配置真实 SERVER_NAME"
  [ -s "$CERTS/fullchain.pem" ] && [ -s "$CERTS/privkey.pem" ] || die "--tls 需要 certs/fullchain.pem 与 certs/privkey.pem"
  USE_TLS=1
elif [ $FORCE_HTTP -eq 1 ]; then
  USE_TLS=0
elif [ -n "$SERVER_NAME" ] && ! placeholder "$SERVER_NAME" \
     && [ -s "$CERTS/fullchain.pem" ] && [ -s "$CERTS/privkey.pem" ]; then
  USE_TLS=1
else
  USE_TLS=0
fi

COMPOSE=(docker compose -f docker-compose.yml)
if [ "$USE_TLS" = 1 ]; then
  COMPOSE+=(-f docker-compose.tls.yml)
  env_set HTTP_PORT 80
  HTTPS_PORT=$(env_get HTTPS_PORT); HTTPS_PORT=${HTTPS_PORT:-443}
  env_set HTTPS_PORT "$HTTPS_PORT"
  info "TLS：$SERVER_NAME（80→443）"
else
  HTTP_PORT=$(env_get HTTP_PORT); HTTP_PORT=${HTTP_PORT:-8081}
  env_set HTTP_PORT "$HTTP_PORT"
  if [ $FORCE_HTTP -eq 0 ] && [ -z "$SERVER_NAME" -o "$(env_get SERVER_NAME)" = "" ]; then
    warn "未配置 SERVER_NAME + certs/，按 HTTP 启动（:${HTTP_PORT}）。域名证书准备好后加 --tls 或直接重跑。"
  fi
  info "HTTP：:${HTTP_PORT}"
fi

if [ "$REGEN_DONE" = 1 ]; then
  info "密钥已重新生成并渲染。确认上面的警告后，再运行 ./compose-up.sh 起栈。"
  exit 0
fi

info "数据目录：$DATA（上传/附件/导出/备份）"
info "初始账号：admin；初始口令在 secrets/initial_admin_password"
exec "${COMPOSE[@]}" up -d "${UP_ARGS[@]}"
