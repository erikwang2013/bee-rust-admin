#!/usr/bin/env bash
# 逐个发布工作区内尚未上传到 crates.io 的 crate。
# 已发布的自动跳过（cargo publish --workspace 做不到这点，会因 "already uploaded" 中断整轮）；
# 被限流时读取服务端给出的 retry 时刻再继续。可随时 Ctrl-C，重跑会接着发。
set -uo pipefail
cd "$(dirname "$0")/.."

UA='bee-rust-publish (erik@erik.xyz)'
MIN_SLEEP=${MIN_SLEEP:-60}

on_crates() { curl -sf -o /dev/null -A "$UA" "https://crates.io/api/v1/crates/$1/$2"; }

list() {
  cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json,sys
for p in json.load(sys.stdin)["packages"]: print(p["name"], p["version"])'
}

while :; do
  pending=0
  while read -r name ver; do
    on_crates "$name" "$ver" && continue
    pending=$((pending + 1))
    echo "==> $name $ver"
    if out=$(cargo publish -p "$name" 2>&1); then
      echo "    ok"
      continue
    fi
    echo "$out" | grep -E '^error|^Caused by|^  ' | tail -5 | sed 's/^/    /'
    when=$(printf '%s\n' "$out" | sed -n 's/.*try again after \(.*GMT\).*/\1/p' | tail -1)
    if [ -n "$when" ] && until_=$(date -d "$when" +%s 2>/dev/null); then
      wait=$(( until_ - $(date +%s) + 15 ))
      [ "$wait" -lt "$MIN_SLEEP" ] && wait=$MIN_SLEEP
      echo "    限流中，等到 $(date -u -d "@$until_" '+%Y-%m-%d %H:%M:%S UTC')（${wait}s）"
      sleep "$wait"
    fi
  done < <(list)
  [ "$pending" -eq 0 ] && { echo "全部发布完成"; exit 0; }
  echo "-- 本轮结束，剩余 $pending 个，60s 后下一轮 --"
  sleep 60
done
