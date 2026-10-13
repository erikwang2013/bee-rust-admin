#!/usr/bin/env bash
# BRA 发版：构建 → strip → 打包 → 产物自检 → gh release create。
#
#   bash scripts/release.sh v2.9.0 --notes-file /tmp/notes-v2.9.0.md
#   bash scripts/release.sh v2.9.0 --notes-file /tmp/notes-v2.9.0.md --dry-run
#   bash scripts/release.sh v2.9.0 --notes-file /tmp/notes-v2.9.0.md --skip-images
#
# 产物口径（**别自己另搞一套**：install.sh 的 download 模式按前两个名字认）：
#   bee_admin                  ELF 可执行，**strip 过** —— 不 strip 是 22MB，发布口径 16.6MB
#   bee-admin-web-dist.tar.gz  前端 dist，tar 根下必须是 ./index.html 与 ./assets/
#   bra-images-$TAG.tar.gz     后端 + 前端镜像（bra-backend:$TAG / bra-web:$TAG 的 docker save）
#   bra-compose-$TAG.tar.gz    离线 compose 包（docker-compose.yml + 预填 .env + app.conf.example
#                              + 一页说明），与镜像包配套给 compose 部署用
# 资产名固定；Release **标题只写 vX.Y.Z**（描述全部写在 notes 文件里）。
#
# CI 只测、不构建产物（.github/workflows/test.yml 里写了「镜像仍在本机构建」），
# 所以这些资产只能从本机这样发出来 —— 这个脚本就是那段手工知识的固化。镜像仍在本机
# 构建、离线分发（只多了「随 Release 附带」这一步），不经过任何镜像仓库/CI。
#
# --dry-run：真的构建、真的打包、真的自检，只在最后一步停下（不 push、不建 release），
#            并把将要执行的 gh 命令原样打出来。
# --skip-images：跳过镜像与 compose 包（本机没有 docker daemon 时的后路），只发前两个。
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

用法：bash scripts/release.sh <tag> --notes-file FILE [--dry-run] [--skip-images]

  <tag>               形如 v2.9.0（拒绝重发：本地与远端都不能已有这个 tag）。
                      允许 v9.9.9-rc1 这样的预发布后缀 —— 带后缀的自动标 --prerelease
                      （不标的话它会被当成正式版、占掉 Latest，有人就装到 rc 上了）。
  --notes-file FILE   Release 说明（标题只写 vX.Y.Z，描述全在这个文件里）
  --dry-run           真构建、真打包、真自检，打印将要执行的 gh 命令后退出
  --skip-images       跳过镜像与 compose 包，只发前两个产物（本机没有 docker daemon
                      时的后路）；镜像包与 compose 包本来是随每个 release 一起发的
  --help              显示本帮助

前置（一个不满足就退出，什么都没发）：
  在 main 分支 · 工作树干净 · HEAD 已推送（== origin/main）· tag 不存在 · gh 已登录
  （镜像与 compose 包另需本机 docker daemon —— 没有就用 --skip-images）

产物（target/release-assets/，不入库）：
  bee_admin                   strip 过的 ELF（22MB → 16.6MB）
  bee-admin-web-dist.tar.gz   前端 dist，根下 ./index.html + ./assets/
  bra-images-vX.Y.Z.tar.gz    后端 + 前端镜像（docker save；--skip-images 时不产出）
  bra-compose-vX.Y.Z.tar.gz   离线 compose 包（compose 文件 + .env + 说明；同上）
EOF
}

# ══════════════════════════════════════════════════════════════════════════
#  参数
# ══════════════════════════════════════════════════════════════════════════
TAG=''; NOTES=''; DRY_RUN=0; SKIP_IMAGES=0
need_val() { [ "$2" -ge 2 ] || die "$1 需要一个值（--help 看用法）"; }
while [ $# -gt 0 ]; do
  case $1 in
    --notes-file)  need_val "$1" $#; NOTES=$2; shift 2;;
    --dry-run)     DRY_RUN=1; shift;;
    --skip-images) SKIP_IMAGES=1; shift;;
    --help|-h)     usage; exit 0;;
    -*)            die "未知参数：$1（--help 看用法）";;
    *)             [ -z "$TAG" ] || die "多余的位置参数：$1（--help 看用法）"; TAG=$1; shift;;
  esac
done

[ -n "$TAG" ] || die "少了版本号。用法：bash $PROG v2.9.0 --notes-file FILE"
# 允许预发布后缀（v2.9.0-rc1）；别的形状一律拒绝 —— 资产地址与 tag 名强绑定
[[ $TAG =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] \
  || die "版本号要形如 v2.9.0（预发布写成 v2.9.0-rc1），给的是：$TAG"
[ -n "$NOTES" ] || die "少了 --notes-file。标题里不写描述，描述全放这个文件"
[ -f "$NOTES" ] && [ -r "$NOTES" ] || die "notes 文件读不到：$NOTES"
NOTES=$(cd -- "$(dirname -- "$NOTES")" && pwd -P)/$(basename -- "$NOTES")

need_cmds=(cargo pnpm strip tar install sha256sum)
# docker/gzip 只在做镜像与 compose 包时用（--skip-images 就不强求）
[ "$SKIP_IMAGES" = 1 ] || need_cmds+=(docker gzip)
for c in "${need_cmds[@]}"; do
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
#  镜像与 compose 离线包
#  产物是「本机构建、离线分发」的一部分 —— 不推任何镜像仓库、不走 CI，只是把两个镜像
#  打进 tar.gz 随 Release 附带。compose 包的 .env 预填 BRA_VERSION=$TAG，用户
#  `docker load -i` 之后 `docker compose up -d`（不带 --build）就能起栈。
#  只有这一节需要 docker daemon；--skip-images 整节跳过（只发前两个产物）。
# ══════════════════════════════════════════════════════════════════════════
compose_pkg=''
trap 'rm -rf "$compose_pkg"' EXIT
if [ "$SKIP_IMAGES" = 1 ]; then
  warn "跳过镜像与 compose 包（--skip-images）：本次只发 bee_admin 与前端 dist"
else
  say "构建镜像（后端用 deploy/Dockerfile.prebuilt 打上面那个 strip 产物、不重编译；前端在容器里 pnpm build）"
  docker info >/dev/null 2>&1 \
    || die "本机没有可用的 docker daemon；离线镜像包需要它（或用 --skip-images 跳过，见 --help）"
  docker build -f deploy/Dockerfile.prebuilt -t "bra-backend:$TAG" . \
    || die "后端镜像构建失败（deploy/Dockerfile.prebuilt）"
  docker build -t "bra-web:$TAG" admin/web \
    || die "前端镜像构建失败（admin/web/Dockerfile）"
  info "bra-backend:$TAG / bra-web:$TAG 构建完成"

  say "导出离线镜像包（docker save | gzip）"
  docker save "bra-backend:$TAG" "bra-web:$TAG" | gzip > "$ASSETS/bra-images-$TAG.tar.gz" \
    || die "docker save / gzip 失败"
  gzip -t "$ASSETS/bra-images-$TAG.tar.gz" || die "bra-images-$TAG.tar.gz 不是完好的 gzip"
  sha_img=$(sha256sum "$ASSETS/bra-images-$TAG.tar.gz" | cut -d' ' -f1)
  info "bra-images-$TAG.tar.gz   $(stat -c%s "$ASSETS/bra-images-$TAG.tar.gz") 字节  sha256 $sha_img"

  say "组 compose 包（compose 文件 + 预填 .env + app.conf.example + 一页 README）"
  compose_pkg=$(mktemp -d)
  pkg="$compose_pkg/bra-compose-$TAG"
  mkdir -p "$pkg/admin/conf"
  cp docker-compose.yml "$pkg/docker-compose.yml"
  cp admin/conf/app.conf.example "$pkg/admin/conf/app.conf.example"
  # .env 与 README 用「引号 heredoc + @TAG@ 占位」生成：内容里全是 markdown 反引号，
  # 不引号的 heredoc 会把它们当命令替换执行掉
  cat > "$pkg/.env" <<'EOF'
# docker compose 用的环境变量（离线包已预填；第一行必须改成你自己的口令）

# compose 内置 MySQL 的 root 密码（必填）
MYSQL_ROOT_PASSWORD=change-me-please

# 镜像 tag：compose 里的 bra-backend / bra-web 就取这个 tag，= 本包 docker load 进来的
# 那个 tag（已预填成 @TAG@，不用改）
BRA_VERSION=@TAG@

# 后端容器以什么身份运行：默认取宿主 uid/gid 1000（即大多数桌面 Linux 的第一个用户），
# 这样它才读得到 600 权限的 admin/conf/app.conf。用 `id -u` / `id -g` 查自己的值。
# APP_UID=1000
# APP_GID=1000
EOF
  cat > "$pkg/README.md" <<'EOF'
# BRA 离线部署包 @TAG@

compose 起栈：MySQL + 后端 + 前端（对外 8081）。不用 Rust / Node 工具链，不联网拉源码。

## 起栈

```bash
# 1) 先载入镜像 —— compose 不带 --build 就用这两个 tag
#    （镜像不在本机时 compose 会想现场 build，而本包没有源码，所以必须先 load）
docker load -i bra-images-@TAG@.tar.gz

# 2) 准备后端配置：这两项必填，示例值会被拒绝启动
#      [app] encrypt_key = base64:<openssl rand -base64 32 的输出>
#      [jwt] secret      = 随机 ≥32 字符
cp admin/conf/app.conf.example admin/conf/app.conf
vi admin/conf/app.conf

# 3) 改 .env 第一行的 MySQL root 口令（预填的 change-me-please 必须换）
vi .env

# 4) 起栈
docker compose up -d

# 打开 http://localhost:8081（初始账号 admin / 配置里的 initial_admin_password）
```

## 两个坑

- **`app.conf` 是 600 权限时**：容器要以「配置文件属主」的身份跑才读得到它，对应
  `.env` 里的 `APP_UID`/`APP_GID`（默认 `1000:1000`）；宿主 uid 不是 1000（`id -u` 查）
  就填上，否则后端起来就报读不到配置。
- **想从源码构建**：加 `--build`（`docker compose up -d --build`）—— 要 Rust/Node
  工具链、慢得多，且构建出的镜像会覆盖同名 tag；这件事要在 git 仓库那份 compose 上做，
  本包不含源码，只能走已 load 的镜像。

## 其它

- 数据在 `mysql-data` 卷里：`docker compose down` 不删数据，`docker compose down -v` 才删
- `mysql:8.4` 由 compose 首次 `up` 时从 Docker Hub 拉；完全离线的环境要自己把这个
  基础镜像弄进来（本包只带 BRA 自己的两个镜像）
- 镜像 build 自本仓库 `@TAG@` 那个提交（后端 = 发布口径 strip 过的二进制）
EOF
  sed -i "s/@TAG@/$TAG/g" "$pkg/.env" "$pkg/README.md"
  tar czf "$ASSETS/bra-compose-$TAG.tar.gz" -C "$compose_pkg" "bra-compose-$TAG" \
    || die "组 compose 包失败"
  for f in docker-compose.yml .env README.md admin/conf/app.conf.example; do
    tar tzf "$ASSETS/bra-compose-$TAG.tar.gz" | grep -qx "bra-compose-$TAG/$f" \
      || die "compose 包里没有 $f"
  done
  rm -rf "$compose_pkg"; compose_pkg=''
  sha_pkg=$(sha256sum "$ASSETS/bra-compose-$TAG.tar.gz" | cut -d' ' -f1)
  info "bra-compose-$TAG.tar.gz $(stat -c%s "$ASSETS/bra-compose-$TAG.tar.gz") 字节  sha256 $sha_pkg"
fi

# ══════════════════════════════════════════════════════════════════════════
#  发布（标题只写 tag；--target 钉住 HEAD，远端 tag 就落在被测过的那个提交上）
# ══════════════════════════════════════════════════════════════════════════
gh_cmd=(gh release create "$TAG" --title "$TAG" --notes-file "$NOTES" --target "$HEAD")
# 带后缀的 tag（v2.9.0-rc1）是预发布：必须标 --prerelease —— 不标就会被当成正式版发布、
# 还会占掉 Latest，而 install.sh 默认模式取 latest，等于把人往 rc 上引。纯 vX.Y.Z 才不加。
case $TAG in *-*) gh_cmd+=(--prerelease);; esac
gh_cmd+=("$ASSETS/bee_admin" "$ASSETS/bee-admin-web-dist.tar.gz")
# 镜像 + compose 包是「离线部署」那套，--skip-images 时它们根本不存在，不能挂进资产
[ "$SKIP_IMAGES" = 1 ] || gh_cmd+=("$ASSETS/bra-images-$TAG.tar.gz" "$ASSETS/bra-compose-$TAG.tar.gz")

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
urls="  https://github.com/erikwang2013/bee-rust-admin/releases/download/$TAG/bee_admin
  https://github.com/erikwang2013/bee-rust-admin/releases/download/$TAG/bee-admin-web-dist.tar.gz"
if [ "$SKIP_IMAGES" = 0 ]; then
  urls="$urls
  https://github.com/erikwang2013/bee-rust-admin/releases/download/$TAG/bra-images-$TAG.tar.gz
  https://github.com/erikwang2013/bee-rust-admin/releases/download/$TAG/bra-compose-$TAG.tar.gz"
fi
cat >&2 <<EOF

${C_GRN}发完了。${C_OFF}资产的下载地址（前两个就是 install.sh 的 download 模式按名拉的那两个）：
$urls

  装一套试试（默认模式 = 拉前两个；glibc 要 ≥ 2.38）：
    bash install.sh --version $TAG
EOF
