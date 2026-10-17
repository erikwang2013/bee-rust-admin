# install.sh · 取产物与可运行性检查（download 模式；落盘之前）。
fetch_artifacts() {
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
  # BRA_VERSION 进页脚站点信息（src/SiteInfo.tsx）：--version 给了就用它，否则显示 latest
  ( cd "$REPO_DIR/admin/web" && pnpm install --frozen-lockfile && BRA_VERSION="$VERSION" pnpm build ) \
    || step_fail "$EXIT_FAIL" "前端构建失败"
  install -m0755 "$REPO_DIR/target/release/bee_admin" "$BIN_NEW"
  tar czf "$TMP/bee-admin-web-dist.tar.gz" -C "$REPO_DIR/admin/web/dist" .
  VERSION_FILE_VALUE="built-$(date -u '+%Y%m%d%H%M%S')"
fi
}

check_runnability() {
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
}
