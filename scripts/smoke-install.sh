#!/usr/bin/env bash
# install.sh 的冒烟验证：在 /tmp 里装一套 scratch 实例 → 断言 → --keep-conf 升级一次 → 断言 → 清理。
#
#   bash scripts/smoke-install.sh
#
# 它复现的是 D6 那轮手工真跑 —— 那次逮到「--keep-conf 升级时 ALTER USER 把库里口令改飞」。
# 改 install.sh 之后跑这个；全绿才算没把升级路径弄坏。
#
# ══════════════════════════════════════════════════════════════════════════
#  护栏：**默认不碰任何生产资源**（想绕过先想清楚）
#  1. 库名/账号名固定 `<DSN用户>_smoke_test`：**固定后缀 + mktemp 前缀 + 收尾残留反查
#     与生产 pid 比对**才是真正的保护；下面「以 _test 结尾 / 不撞生产库」两道在构造上
#     恒真（兜底断言：将来谁改了命名规则立刻炸）。`ADMIN_USER`/`GRANT_HOST` 拼进
#     DROP SQL 前先过字符集白名单 —— 来源虽是操作者自己的 app.conf，照样不许改写语句。
#  2. 端口固定 180xx（生产 8080 一眼可分）；服务名 bra-smoke-<pid>（**必须**：
#     测试脚本永远不许碰名叫 bee-admin 的单元 —— 换单元名是最后一道护栏。install.sh
#     第 9 步的 `systemctl restart $SERVICE_NAME` 曾经不受 --no-systemd 约束、有 root 时
#     就会去 restart 生产单元；现已加门控（--no-systemd 直接跳过重启）。
#  3. 前缀是 mktemp 出来的 /tmp/bra-smoke-*，删它之前还要再验证一次路径形状。
#  4. 全程只读 admin/conf/app.conf 拿管理员凭据（连接用 MYSQL_PWD，不进 argv、不打印）。
# ══════════════════════════════════════════════════════════════════════════
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "$0")/.." 2>/dev/null && pwd -P) || exit 1
INSTALL=$ROOT/install.sh
CONF=$ROOT/admin/conf/app.conf
PROG=${0##*/}

# ── 生产资源的名字：下面任何命令都不许用它们 ──
PROD_PORT=8080
PROD_DB=bee_admin
PROD_TEST_DB=bee_admin_test
PROD_SERVICE=bee-admin

# ── scratch 参数（固定值，输出里一眼看出是冒烟） ──
HTTP_PORT=18081
BACKEND_PORT=18080
SERVICE_NAME="bra-smoke-$$"

if [ -t 2 ]; then
  C_RED=$'\033[31m'; C_YEL=$'\033[33m'; C_GRN=$'\033[32m'; C_BLD=$'\033[1m'; C_OFF=$'\033[0m'
else
  C_RED=''; C_YEL=''; C_GRN=''; C_BLD=''; C_OFF=''
fi
say()  { printf '%s==>%s %s\n' "$C_BLD" "$C_OFF" "$*" >&2; }
info() { printf '    %s\n' "$*" >&2; }
warn() { printf '%s警告:%s %s\n' "$C_YEL" "$C_OFF" "$*" >&2; }
die()  { printf '%s错误:%s %s\n' "$C_RED" "$C_OFF" "$*" >&2; exit 1; }

[ -f "$INSTALL" ] || die "找不到 $INSTALL"
[ -f "$CONF" ]    || die "找不到 $CONF（要先有一份真配置才能拿管理员凭据）"
# 收尾的 DROP 要用它；没装的话现在就说，别等清到一半才发现
command -v mysql >/dev/null 2>&1 || die "本机没有 mysql 客户端（install.sh 与收尾清理都要用）"

# ══════════════════════════════════════════════════════════════════════════
#  0. 生产实例的 pid（收尾时比对；开工前就把名字与端口钉死）
# ══════════════════════════════════════════════════════════════════════════
prod_pid() { # $1=端口 → 监听该端口的进程 pid（取第一个）
  # ⚠️ awk 里**不能**用 exit 收尾：那会让 ss 吃 SIGPIPE(141)，配上 set -o pipefail
  #    整条管道被判失败，脚本还没干活就「莫名其妙」退出（本脚本第一版就这么死的）。
  #    用标志位只打第一条，让 awk 把输入读完。
  ss -ltnp 2>/dev/null | awk -v p=":$1" \
    '!done && $4 ~ (p "$") && match($0, /pid=[0-9]+/) { print substr($0, RSTART + 4, RLENGTH - 4); done = 1 }'
}
PROD_PID_BEFORE=$(prod_pid "$PROD_PORT")
if [ -n "$PROD_PID_BEFORE" ]; then
  info "记录：$PROD_PORT 生产实例 pid=$PROD_PID_BEFORE（全程不许变）"
else
  warn "$PROD_PORT 上没有监听进程 —— 收尾时会要求它「还是空的」"
fi

# ══════════════════════════════════════════════════════════════════════════
#  1. 从 app.conf 取管理员凭据（root 建库用；口令不打印、不进 argv）
# ══════════════════════════════════════════════════════════════════════════
say "从 $CONF 读 [db] dsn"
# urlenc 的逆（install.sh 写 DSN 时会编码）；%25 必须**最后**换，否则字面 "%3A" 会被误解成冒号
urldec() {
  printf '%s' "$1" | sed -e 's/%3A/:/g' -e 's|%2F|/|g' -e 's/%23/#/g' -e 's/%3F/?/g' \
    -e 's/%40/@/g' -e 's/%26/\&/g' -e 's/%2B/+/g' -e 's/%25/%/g'
}
# 不接 `| head -1`：head 提前退出会让 sed 吃 SIGPIPE(141)，pipefail 下整条管道算失败
dsn_lines=$(sed -n 's|^[[:space:]]*dsn[[:space:]]*=[[:space:]]*mysql://||p' "$CONF")
dsn=${dsn_lines%%$'\n'*}
[ -n "$dsn" ] || die "$CONF 里没有 mysql:// 形式的 [db] dsn"
userpass=${dsn%%@*}; hostpart=${dsn#*@}
case $userpass in *:*) ;; *) die "[db] dsn 里没有口令（形如 mysql://用户:口令@主机/库）" ;; esac
ADMIN_USER=$(urldec "${userpass%%:*}")
ADMIN_PASS=$(urldec "${userpass#*:}")
hostport=${hostpart%%/*}
DB_HOST=${hostport%%:*}; DB_PORT=${hostport##*:}
if [ "$hostport" = "$DB_HOST" ]; then DB_PORT=3306; fi   # dsn 里没写端口
info "管理员账号：$ADMIN_USER@$DB_HOST:$DB_PORT（口令已读入，不打印）"

# 建库建号时 install.sh 用的授权主机名（照抄它的规则，DROP 时才找得回那行）
GRANT_HOST=$DB_HOST
case $GRANT_HOST in 127.0.0.1|localhost) GRANT_HOST=127.0.0.1 ;; esac

# ══════════════════════════════════════════════════════════════════════════
#  2. scratch 命名 + 护栏（对不上就退出，什么都不做）
# ══════════════════════════════════════════════════════════════════════════
# 拼进 DROP DATABASE/USER 的两个串先做白名单（反引号/引号就能改写语句；来源是 app.conf，仍不收）
case ${ADMIN_USER:-} in
  ''|*[!A-Za-z0-9_]*) die "护栏：dsn 里的用户名只能含字母数字下划线（当前「${ADMIN_USER:-}」）—— 它要拼进 DROP 语句" ;;
esac
case $GRANT_HOST in
  ''|*[!A-Za-z0-9_.:%-]*) die "护栏：授权主机名字符集异常（当前「$GRANT_HOST」）" ;;
esac
SCRATCH_DB=${ADMIN_USER}_smoke_test
SCRATCH_USER=$SCRATCH_DB
case $SCRATCH_DB in
  *_test) ;;   # 构造上恒真，留着当断言：改了命名规则就在这里炸
  *) die "护栏：scratch 库名必须以 _test 结尾（当前 $SCRATCH_DB）—— 它后面是要被 DROP 的" ;;
esac
for n in "$PROD_DB" "$PROD_TEST_DB"; do
  [ "$SCRATCH_DB" != "$n" ] || die "护栏：scratch 库名撞上生产/测试库（$n）"
done
[ "$SERVICE_NAME" != "$PROD_SERVICE" ] || die "护栏：服务名撞上生产单元"
for p in "$HTTP_PORT" "$BACKEND_PORT"; do
  if [ "$p" -lt 18000 ] || [ "$p" -gt 18999 ]; then
    die "护栏：scratch 端口要在 18000-18999（现在是 $p）—— 离生产 $PROD_PORT 远一点"
  fi
done

WORK=$(mktemp -d "${TMPDIR:-/tmp}/bra-smoke-XXXXXX")
PREFIX=$WORK/prefix
LOG1=$WORK/install-1.log
LOG2=$WORK/install-2.log
DB_PASS=$(openssl rand -hex 16)
info "scratch：库/账号 $SCRATCH_DB · 端口 $HTTP_PORT → $BACKEND_PORT · 前缀 $PREFIX"

# ══════════════════════════════════════════════════════════════════════════
#  3. 清理（trap：失败也清；清完再比对生产 pid）
# ══════════════════════════════════════════════════════════════════════════
mysql_admin() { MYSQL_PWD=$ADMIN_PASS mysql -h"$DB_HOST" -P"$DB_PORT" -u"$ADMIN_USER" "$@"; }

cleanup() {
  local rc=$? pid_after left
  trap - EXIT INT TERM
  set +e
  printf '\n%s==>%s 收尾\n' "$C_BLD" "$C_OFF" >&2

  # 前缀与日志都在这一个 mktemp 出来的目录里（失败路径的日志在上面已经 tail 过）
  case $WORK in
    */bra-smoke-*) rm -rf -- "$WORK" && info "已删 $WORK（前缀 + 日志）";;
    *) warn "路径形状不对，**没删**：$WORK";;
  esac

  printf 'DROP DATABASE IF EXISTS `%s`;\nDROP USER IF EXISTS '\''%s'\''@'\''%s'\'';\nFLUSH PRIVILEGES;\n' \
    "$SCRATCH_DB" "$SCRATCH_USER" "$GRANT_HOST" | mysql_admin >/dev/null 2>&1
  left=$(mysql_admin -N -e "SELECT CONCAT(
      (SELECT COUNT(*) FROM information_schema.SCHEMATA WHERE SCHEMA_NAME='$SCRATCH_DB'), '/',
      (SELECT COUNT(*) FROM mysql.user WHERE user='$SCRATCH_USER'))" 2>/dev/null)
  if [ "$left" = "0/0" ]; then
    info "已清：库 $SCRATCH_DB 与账号 $SCRATCH_USER 残留 0/0"
  else
    warn "清理不干净（库/账号残留 $left）—— 手工来：
    DROP DATABASE IF EXISTS \`$SCRATCH_DB\`; DROP USER IF EXISTS '$SCRATCH_USER'@'$GRANT_HOST';"
    rc=1
  fi

  pid_after=$(prod_pid "$PROD_PORT")
  if [ "$pid_after" = "$PROD_PID_BEFORE" ]; then
    info "8080 生产实例 pid 未变（${pid_after:-还是空的}）"
  else
    warn "8080 生产实例 pid 变了：开工前 ${PROD_PID_BEFORE:-空} → 现在 ${pid_after:-空} —— 生产被动过！"
    rc=1
  fi

  [ "$rc" = 0 ] && printf '\n%s冒烟全绿。%s\n' "$C_GRN$C_BLD" "$C_OFF" >&2
  exit "$rc"
}
trap cleanup EXIT INT TERM

# ══════════════════════════════════════════════════════════════════════════
#  4. 参数（两次运行共用；--keep-conf 只在第二次追加）
# ══════════════════════════════════════════════════════════════════════════
ARGS=(--mode build --yes
      --prefix "$PREFIX" --service-name "$SERVICE_NAME"
      --no-systemd --no-nginx
      --http-port "$HTTP_PORT" --backend-port "$BACKEND_PORT"
      --snowflake 9,0
      --db-host "$DB_HOST" --db-port "$DB_PORT"
      --db-name "$SCRATCH_DB" --db-user "$SCRATCH_USER" --db-pass "$DB_PASS"
      --db-admin-user "$ADMIN_USER")
# 管理员口令**不进 argv**（argv 里 `ps` 看得见）：走 install.sh 支持的环境变量入口
# （install.sh:139 的 BRA_DB_ADMIN_PASS），与头注释「口令不打印、不进 argv」的承诺一致。

want_line() { # $1=日志 $2=必须出现的整串 $3=说明
  local line
  line=$(grep -m1 -F -- "$2" "$1") || die "$3：$1 里找不到「$2」"
  info "$3：$(printf '%s' "$line" | sed 's/^[[:space:]]*//')"
}
run_install() { # $1=日志 $2...=额外参数；返回 install.sh 的退出码
  local log=$1; shift
  local rc=0
  BRA_DB_ADMIN_PASS="$ADMIN_PASS" bash "$INSTALL" "${ARGS[@]}" "$@" >"$log" 2>&1 || rc=$?
  printf '%s' "$rc"
}

# ══════════════════════════════════════════════════════════════════════════
#  5. 第一次：全新安装（含 install.sh 自己的探活自检）
# ══════════════════════════════════════════════════════════════════════════
say "① 全新安装（--mode build，装到 $PREFIX）"
rc1=$(run_install "$LOG1")
if [ "$rc1" != 0 ]; then
  warn "install.sh 第 1 次退出码 $rc1，它最后说的话："
  tail -n 30 "$LOG1" | sed 's/^/    /' >&2
  die "全新安装没通过"
fi
info "退出码 0"

for f in bin/bee_admin VERSION conf/app.conf web/dist/index.html; do
  [ -s "$PREFIX/$f" ] || die "装完缺文件：$PREFIX/$f"
done
info "bin/bee_admin · VERSION（$(cat "$PREFIX/VERSION")）· conf/app.conf · web/dist/index.html 齐全"
want_line "$LOG1" "后端健康检查通过" "探活"

# ══════════════════════════════════════════════════════════════════════════
#  6. 第二次：--keep-conf 升级（D6 那轮逮到 bug 的地方）
#     断言三件事：退出 0 · 日志里 ALTER USER 出现 0 次 · conf 逐字节不变
# ══════════════════════════════════════════════════════════════════════════
cp -p "$PREFIX/conf/app.conf" "$WORK/app.conf.before"
say "② --keep-conf 升级重跑（专抓「升级把库里口令改飞」）"
rc2=$(run_install "$LOG2" --keep-conf)
if [ "$rc2" != 0 ]; then
  warn "install.sh 第 2 次退出码 $rc2，它最后说的话："
  tail -n 30 "$LOG2" | sed 's/^/    /' >&2
  die "--keep-conf 升级没通过"
fi
info "退出码 0"

n_alter=$(grep -c -F -- 'ALTER USER' "$LOG2" || true)
if [ "$n_alter" != 0 ]; then
  grep -n -F -- 'ALTER USER' "$LOG2" | sed 's/^/    /' >&2
  die "--keep-conf 竟然跑了 $n_alter 次 ALTER USER（就是把库里口令改飞的那条路）"
fi
info "ALTER USER：0 次（升级没去动库里的账号）"

# **别用 diff：这台机器上的 diff 是坏的（文件不同也退 0）** —— cmp -s 逐字节比
if ! cmp -s "$WORK/app.conf.before" "$PREFIX/conf/app.conf"; then
  die "--keep-conf 之后 conf 变了：$WORK/app.conf.before 与 $PREFIX/conf/app.conf（encrypt_key 一换，库里数据就解不开）"
fi
info "conf 逐字节不变（cmp -s 通过）"
want_line "$LOG2" "后端健康检查通过" "升级后探活"

say "全绿：全新安装 + --keep-conf 升级都过（收尾见下）"
