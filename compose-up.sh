#!/usr/bin/env bash
# docker compose 一键起栈：必填但没填的口令 / 密钥自动生成，写回本地文件，然后
# `docker compose up -d "$@"`（参数原样转给 up，比如 --build）。
#
#   ./compose-up.sh             # 起栈；缺的自动生成，已填的一律不动
#   ./compose-up.sh --build     # 现场从源码构建（要 Rust/Node 工具链，慢）
#
# 生成的值写回两处本地文件（就是 compose 实际读取的位置，「本地保存一份」即它们）：
#   .env                 MYSQL_ROOT_PASSWORD —— compose 的 mysql 与后端 DSN 共用这个值
#   admin/conf/app.conf  [app] encrypt_key / hashids_salt、[jwt] secret、
#                        [seed] initial_admin_password
# 只替换「空 / 缺键 / 示例占位符」；填过真值的不动，重复跑不会重新生成。
#
# ⚠️ 生成后请备份这两个文件：encrypt_key 丢了（或换了），库里已加密的 email/phone
#    就永久解不开；写回前是占位符的话没有这个风险，生成后才有。
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "$0")" 2>/dev/null && pwd -P) || exit 1
cd "$ROOT"

ENV_FILE=.env
ENV_EXAMPLE=.env.example
CONF=admin/conf/app.conf
CONF_EXAMPLE=admin/conf/app.conf.example

info() { printf '    %s\n' "$*" >&2; }
die()  { printf '错误: %s\n' "$*" >&2; exit 1; }

command -v openssl >/dev/null 2>&1 \
  || die "需要 openssl 生成随机值（发行版一般自带；没有就手动填好 $ENV_FILE 与 $CONF 再跑 docker compose）"

# ── .env：MYSQL_ROOT_PASSWORD ────────────────────────────────────────────
[ -f "$ENV_FILE" ] || { cp "$ENV_EXAMPLE" "$ENV_FILE"; info "已从 $ENV_EXAMPLE 创建 $ENV_FILE"; }
cur=$(sed -n 's/^MYSQL_ROOT_PASSWORD[[:space:]]*=//p' "$ENV_FILE" | head -1 || true)
if [ -z "$cur" ] || [ "$cur" = "change-me-please" ]; then
  # 纯 [0-9a-f]：这个值要塞进 DSN（mysql://root:PW@mysql:3306/...），别的字符要编码
  pw=$(openssl rand -hex 16)
  if grep -q '^MYSQL_ROOT_PASSWORD[[:space:]]*=' "$ENV_FILE"; then
    sed -i "s#^MYSQL_ROOT_PASSWORD[[:space:]]*=.*#MYSQL_ROOT_PASSWORD=$pw#" "$ENV_FILE"
  else
    printf 'MYSQL_ROOT_PASSWORD=%s\n' "$pw" >> "$ENV_FILE"
  fi
  info "已生成 MYSQL_ROOT_PASSWORD → $ENV_FILE"
fi

# ── admin/conf/app.conf ──────────────────────────────────────────────────
[ -f "$CONF" ] || { cp "$CONF_EXAMPLE" "$CONF"; info "已从 $CONF_EXAMPLE 创建 $CONF"; }

get_ini() { sed -n "s/^$1[[:space:]]*=[[:space:]]*//p" "$CONF" | head -1 || true; }
set_ini() { # key value section —— 键在就换值，不在就插在该节头后面
  if grep -q "^$1[[:space:]]*=" "$CONF"; then
    sed -i "s#^$1[[:space:]]*=.*#$1 = $2#" "$CONF"
  else
    sed -i "/^\[$3\][[:space:]]*$/a $1 = $2" "$CONF"
  fi
}

# encrypt_key：判据与后端启动校验同口径（空 / 含 change-me 都会被拒启）
cur=$(get_ini encrypt_key)
case "$cur" in
  "" | *change-me*) set_ini encrypt_key "base64:$(openssl rand -base64 32)" app
                    info "已生成 [app] encrypt_key → $CONF";;
esac

# [jwt] secret：空 / changeme / 示例占位符（示例那串 36 字符能过启动校验，但人尽皆知）
cur=$(get_ini secret)
case "$cur" in
  "" | changeme | change-me-to-a-random-string-of-32-chars)
      set_ini secret "$(openssl rand -hex 32)" jwt
      info "已生成 [jwt] secret → $CONF";;
esac

# hashids_salt：只替换示例占位符 —— 空值可能是刻意保留的（换盐 = 已发出的短串作废）
cur=$(get_ini hashids_salt)
case "$cur" in
  change-me-to-a-random-salt) set_ini hashids_salt "$(openssl rand -hex 16)" app
                              info "已生成 [app] hashids_salt → $CONF";;
esac

# initial_admin_password：空 / 示例占位符。Pw+16hex：位数与字符类别都过后端种子校验
cur=$(get_ini initial_admin_password)
case "$cur" in
  "" | change-me-please-1)
      admin_pw="Pw$(openssl rand -hex 8)"
      set_ini initial_admin_password "$admin_pw" seed
      info "已生成 [seed] initial_admin_password → $CONF"
      info "初始账号 admin 的初始口令：$admin_pw（仅在首次建库时生效；登录后立即改）";;
esac

# 最后确认一次：走到这里 encrypt_key 不能还是空/占位符，否则后端会拒启
cur=$(get_ini encrypt_key)
case "$cur" in
  "" | *change-me*) die "$CONF 里的 encrypt_key 仍是空或占位符（节头或键行被改坏了？），请手动检查";;
esac

exec docker compose up -d "$@"
