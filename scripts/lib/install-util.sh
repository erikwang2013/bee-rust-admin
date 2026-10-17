# install.sh · 小工具（参数转义 / URL 编码 / 随机口令 / INI / 端口与 root 判定）。
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
