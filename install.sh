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
#
# 实现拆分：本文件只做入口（加载 lib、解析参数、按顺序编排步骤）。各步骤在
# `scripts/lib/install-*.sh`，因此需要在仓库里运行（或至少让 `scripts/lib/` 与本文件
# 保持相对位置）；旧的 `curl .../install.sh | bash` 单文件用法请改为下载仓库后运行。
set -euo pipefail
umask 022
ORIG_ARGS=("$@")   # 参数解析会把 $@ 吃光，后面要拿它拼「重跑」提示

PROG=${0##*/}
EXIT_OK=0; EXIT_FAIL=1; EXIT_PREFLIGHT=2; EXIT_NEED_ROOT=3
EXIT_NEED_DB=4; EXIT_SELFTEST=5; EXIT_CONF_EXISTS=6

# 脚本自身目录：库与仓库根都相对它定位（BASH_SOURCE 在 bash 下比 $0 稳）
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd -P || echo "")
LIB_DIR=$SCRIPT_DIR/scripts/lib

load_lib() { # $1 = 库文件名
  local f=$LIB_DIR/$1
  [ -f "$f" ] || {
    printf '错误: 缺脚本依赖 %s\n' "$f" >&2
    printf '      入口需要同仓库的 scripts/lib/；curl 单文件用法请改为下载仓库后再跑 install.sh。\n' >&2
    exit "$EXIT_FAIL"
  }
  . "$f"
}
for lib in install-common.sh install-util.sh install-usage.sh install-args.sh \
           install-templates.sh install-preflight.sh install-fetch.sh \
           install-payload.sh install-services.sh install-finish.sh; do
  load_lib "$lib"
done

# 流程编排：与原单文件实现的执行顺序一一对应。
parse_args "$@"
dry_run
preflight
guard_conf
prepare_secrets
fetch_artifacts
check_runnability
install_payload
prepare_database
install_services
start_and_selfcheck
finish

# 还有 root 才能做的步骤没做（systemd / nginx）→ 退 3，把「装完了」和「还差一步」区分开
if [ -n "$need_root_cmds" ]; then
  printf '%s还差最后一步%s：上面那两条 sudo 命令（跑完就真的完事了）。\n\n' "$C_YEL" "$C_OFF" >&2
  exit "$EXIT_NEED_ROOT"
fi
exit "$EXIT_OK"
