# install.sh · 输出与错误出口（被 install.sh source；EXIT_* 由入口先定义）。
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
