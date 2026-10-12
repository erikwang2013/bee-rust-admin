#!/usr/bin/env bash
# BRA 发版：构建 → strip → 打包 → 产物自检 → gh release create。
#
#   bash scripts/release.sh v2.9.0 --notes-file /tmp/notes-v2.9.0.md
#   bash scripts/release.sh v2.9.0 --notes-file /tmp/notes-v2.9.0.md --dry-run
#
# 产物口径（**别自己另搞一套**：install.sh 的 download 模式按这两个名字认）：
#   bee_admin                  ELF 可执行，**strip 过** —— 不 strip 是 22MB，发布口径 16.6MB
#   bee-admin-web-dist.tar.gz  前端 dist，tar 根下必须是 ./index.html 与 ./assets/
# 资产名固定；Release **标题只写 vX.Y.Z**（描述全部写在 notes 文件里）。
#
# CI 只测、不构建产物（.github/workflows/test.yml 里写了「镜像仍在本机构建」），
# 所以这两个资产只能从本机这样发出来 —— 这个脚本就是那段手工知识的固化。
#
# --dry-run：真的构建、真的打包、真的自检，只在最后一步停下（不 push、不建 release），
#            并把将要执行的 gh 命令原样打出来。
set -euo pipefail

PROG=${0##*/}
ROOT=$(cd -- "$(dirname -- "$0")/.." 2>/dev/null && pwd -P) || exit 1
cd "$ROOT"

# 空目录，随用随建；target/ 在 .gitignore 里，产物永远不会被 git 看上
ASSETS=target/release-assets
BIN_SRC=target/release/bee_admin
DIST_SRC=admin/web/dist

if [ -t 2 ]; then
  C_RED=$'\033[31m'; C_YEL=$'\033[33m'; C_GRN=$'\033[32m'; C_BLD=$'\033[1m'; C_OFF=$'\033[0m'
else
  C_RED=''; C_YEL=''; C_GRN=''; C_BLD=''; C_OFF=''
fi
say()  { printf '%s==>%s %s\n' "$C_BLD" "$C_OFF" "$*" >&2; }
info() { printf '    %s\n' "$*" >&2; }
warn() { printf '%s警告:%s %s\n' "$C_YEL" "$C_OFF" "$*" >&2; }
die()  { printf '%s错误:%s %s\n' "$C_RED" "$C_OFF" "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }
print_cmd() { local a; for a in "$@"; do printf '%q ' "$a"; done; printf '\n'; }

usage() {
  cat <<'EOF'
BRA 发版脚本

用法：bash scripts/release.sh <tag> --notes-file FILE [--dry-run]

  <tag>               形如 v2.9.0（拒绝重发：本地与远端都不能已有这个 tag）。
                      允许 v9.9.9-rc1 这样的预发布后缀 —— 带后缀的自动标 --prerelease
                      （不标的话它会被当成正式版、占掉 Latest，有人就装到 rc 上了）。
  --notes-file FILE   Release 说明（标题只写 vX.Y.Z，描述全在这个文件里）
  --dry-run           真构建、真打包、真自检，打印将要执行的 gh 命令后退出
  --help              显示本帮助

前置（一个不满足就退出，什么都没发）：
  在 main 分支 · 工作树干净 · HEAD 已推送（== origin/main）· tag 不存在 · gh 已登录

产物（target/release-assets/，不入库）：
  bee_admin                   strip 过的 ELF（22MB → 16.6MB）
  bee-admin-web-dist.tar.gz   前端 dist，根下 ./index.html + ./assets/
EOF
}

# ══════════════════════════════════════════════════════════════════════════
#  参数
# ══════════════════════════════════════════════════════════════════════════
TAG=''; NOTES=''; DRY_RUN=0
need_val() { [ "$2" -ge 2 ] || die "$1 需要一个值（--help 看用法）"; }
while [ $# -gt 0 ]; do
  case $1 in
    --notes-file) need_val "$1" $#; NOTES=$2; shift 2;;
    --dry-run)    DRY_RUN=1; shift;;
    --help|-h)    usage; exit 0;;
    -*)           die "未知参数：$1（--help 看用法）";;
    *)            [ -z "$TAG" ] || die "多余的位置参数：$1（--help 看用法）"; TAG=$1; shift;;
  esac
done

[ -n "$TAG" ] || die "少了版本号。用法：bash $PROG v2.9.0 --notes-file FILE"
# 允许预发布后缀（v2.9.0-rc1）；别的形状一律拒绝 —— 资产地址与 tag 名强绑定
[[ $TAG =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] \
  || die "版本号要形如 v2.9.0（预发布写成 v2.9.0-rc1），给的是：$TAG"
[ -n "$NOTES" ] || die "少了 --notes-file。标题里不写描述，描述全放这个文件"
[ -f "$NOTES" ] && [ -r "$NOTES" ] || die "notes 文件读不到：$NOTES"
NOTES=$(cd -- "$(dirname -- "$NOTES")" && pwd -P)/$(basename -- "$NOTES")

for c in cargo pnpm strip tar install sha256sum; do
  have "$c" || die "缺少命令：$c"
done

# ══════════════════════════════════════════════════════════════════════════
#  前置检查（跑完再构建；一个不满足就别浪费二十分钟）
#  干跑时降级成警告 —— 干跑的意义就是「先看看」，但会明确列出真发版会挡在哪
# ══════════════════════════════════════════════════════════════════════════
say "前置检查"
problems=()
add_problem() { problems+=("$1"); }

branch=$(git rev-parse --abbrev-ref HEAD)
[ "$branch" = main ] || add_problem "当前在 $branch 分支，发版要在 main 上（git switch main）"

dirty=$(git status --porcelain)
if [ -n "$dirty" ]; then
  add_problem "工作树不干净（构建出来的东西会与 tag 指向的提交对不上）：
$(printf '%s\n' "$dirty" | sed 's/^/        /')"
fi

HEAD=$(git rev-parse HEAD)
origin_main=$(git rev-parse origin/main 2>/dev/null || echo '')
if [ -z "$origin_main" ]; then
  add_problem "没有 origin/main 这个引用（git fetch origin 再看看）"
elif [ "$HEAD" != "$origin_main" ]; then
  add_problem "HEAD（${HEAD:0:12}）≠ origin/main（${origin_main:0:12}）—— 先把 $branch 推上去（git push），别发一个源码不在远端的版本"
fi

if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  add_problem "本地已有 tag $TAG（发过的版本不要重发；真要重来先 git tag -d $TAG）"
fi
remote_tags=$(git ls-remote --tags origin "refs/tags/$TAG" 2>/dev/null) \
  || add_problem "查不到远端 tag（网络？git ls-remote --tags origin 手工试一次）"
[ -z "$remote_tags" ] || add_problem "远端已有 tag $TAG（远端才是事实：换版本号）"

if ! have gh; then
  add_problem "没有 gh（GitHub CLI）：https://cli.github.com"
elif ! gh auth status >/dev/null 2>&1; then
  add_problem "gh 未登录（gh auth login）"
fi

if [ "${#problems[@]}" -gt 0 ]; then
  if [ "$DRY_RUN" = 1 ]; then
    warn "干跑：下面这几条真发版时会直接退出（干跑继续，把构建与打包验完）"
    for p in "${problems[@]}"; do printf '%s    · %s\n' "$C_YEL" "$p" >&2; done
  else
    printf '\n%s[中止]%s 前置检查没过，什么都没做：\n' "$C_RED" "$C_OFF" >&2
    for p in "${problems[@]}"; do printf '    · %s\n' "$p" >&2; done
    exit 1
  fi
else
  info "在 main、工作树干净、HEAD 已推送、tag $TAG 未用过、gh 已登录"
fi

# ══════════════════════════════════════════════════════════════════════════
#  构建
# ══════════════════════════════════════════════════════════════════════════
say "构建后端（这一步慢，几分钟到几十分钟；机器忙的时候更慢，别急着按 Ctrl-C）"
( cd "$ROOT" && cargo build --release -p bee_admin ) || die "cargo 构建失败"
[ -x "$BIN_SRC" ] || die "$BIN_SRC 没生成"

say "构建前端（pnpm install --frozen-lockfile + pnpm build）"
( cd "$ROOT/admin/web" && pnpm install --frozen-lockfile && pnpm build ) || die "前端构建失败"
[ -f "$DIST_SRC/index.html" ] || die "$DIST_SRC/index.html 没生成"

# ══════════════════════════════════════════════════════════════════════════
#  打包（strip 是口径的一部分：install.sh 的 download 模式按这两个名字拉）
# ══════════════════════════════════════════════════════════════════════════
say "打包到 $ASSETS/"
rm -rf "$ASSETS"
mkdir -p "$ASSETS"
size_before=$(stat -c%s "$BIN_SRC")
install -m0755 "$BIN_SRC" "$ASSETS/bee_admin"
strip "$ASSETS/bee_admin"
size_after=$(stat -c%s "$ASSETS/bee_admin")
info "bee_admin：$size_before → $size_after 字节（strip 掉符号表）"
tar czf "$ASSETS/bee-admin-web-dist.tar.gz" -C "$DIST_SRC" .
info "bee-admin-web-dist.tar.gz：$(stat -c%s "$ASSETS/bee-admin-web-dist.tar.gz") 字节"

# ══════════════════════════════════════════════════════════════════════════
#  产物自检（判据与 install.sh 第 5 步一致，别在这里另发明一套）
# ══════════════════════════════════════════════════════════════════════════
say "产物自检"
# 让 stripped 二进制真的跑一次：能走到配置层就一定打出 `Error: Missing …`
# （用「正面特征」判定，不查报错文字 —— 加载失败的花样太多，有的一个字都不打）
probe_out=$(BEE_ADMIN_CONF=/dev/null "$ASSETS/bee_admin" 2>&1) || true
case $probe_out in
  *"Error: Missing"*) info "二进制能加载、能走到配置层" ;;
  *) die "stripped 二进制跑不起来，它只说了这些（前 3 行）：
$(printf '%s\n' "${probe_out:-<无输出>}" | sed -n '1,3p' | sed 's/^/    /')" ;;
esac

tar_list=$(tar tzf "$ASSETS/bee-admin-web-dist.tar.gz")
case $tar_list in *'./index.html'*) ;; *) die "前端 tar 里没有 ./index.html（install.sh 靠它认产物）";; esac
case $tar_list in *'./assets/'*)    ;; *) die "前端 tar 里没有 ./assets/";; esac
info "前端 tar 根下有 ./index.html 与 ./assets/"

sha_bin=$(sha256sum "$ASSETS/bee_admin" | cut -d' ' -f1)
sha_dist=$(sha256sum "$ASSETS/bee-admin-web-dist.tar.gz" | cut -d' ' -f1)
info "bee_admin                 $size_after 字节  sha256 $sha_bin"
info "bee-admin-web-dist.tar.gz $(stat -c%s "$ASSETS/bee-admin-web-dist.tar.gz") 字节  sha256 $sha_dist"

# ══════════════════════════════════════════════════════════════════════════
#  发布（标题只写 tag；--target 钉住 HEAD，远端 tag 就落在被测过的那个提交上）
# ══════════════════════════════════════════════════════════════════════════
gh_cmd=(gh release create "$TAG" --title "$TAG" --notes-file "$NOTES" --target "$HEAD")
# 带后缀的 tag（v2.9.0-rc1）是预发布：必须标 --prerelease —— 不标就会被当成正式版发布、
# 还会占掉 Latest，而 install.sh 默认模式取 latest，等于把人往 rc 上引。纯 vX.Y.Z 才不加。
case $TAG in *-*) gh_cmd+=(--prerelease);; esac
gh_cmd+=("$ASSETS/bee_admin" "$ASSETS/bee-admin-web-dist.tar.gz")

if [ "$DRY_RUN" = 1 ]; then
  say "干跑结束：没有推送、没有建 release、没动远端。将要执行的命令是："
  printf '\n    %s\n\n' "$(print_cmd "${gh_cmd[@]}")"
  printf '%s产物留在 %s/（要重新生成就重跑本脚本；要清掉 rm -rf %s）%s\n' "$C_GRN" "$ASSETS" "$ASSETS" "$C_OFF" >&2
  exit 0
fi

say "创建 Release $TAG"
"${gh_cmd[@]}" || die "gh release create 失败（看上面的输出；远端已有 tag？）"

say "回显资产清单（最后确认一次）"
gh release view "$TAG"
cat >&2 <<EOF

${C_GRN}发完了。${C_OFF}两个资产的下载地址（install.sh 的 download 模式就按它们拉）：
  https://github.com/erikwang2013/bee-rust-admin/releases/download/$TAG/bee_admin
  https://github.com/erikwang2013/bee-rust-admin/releases/download/$TAG/bee-admin-web-dist.tar.gz

  装一套试试（默认模式 = 拉上面这两个；glibc 要 ≥ 2.38）：
    bash install.sh --version $TAG
EOF
