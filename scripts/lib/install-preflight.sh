# install.sh · 干跑 / 预检 / 已有配置保护 / 密钥与交互补齐（不写盘或写盘前）。
dry_run() {
if [ "$DRY_RUN" = 1 ]; then
  say "干跑（--dry-run）：以下内容**不会**落盘"
  info "模式：$MODE   版本：$VERSION   安装根：$PREFIX"
  info "服务名：$SERVICE_NAME   对外端口：$HTTP_PORT   后端端口：$BACKEND_PORT"
  info "数据库：$DB_USER@$DB_HOST:$DB_PORT/$DB_NAME"
  if [ "$MODE" = download ]; then
    info "产物地址：$BASE_URL/$VERSION/bee_admin（以及 bee-admin-web-dist.tar.gz）"
  else
    info "源码目录：$REPO_DIR（cargo build --release + pnpm build）"
  fi
  printf '\n%s──── conf/app.conf（将要写入）────%s\n' "$C_BLD" "$C_OFF" >&2
  [ -n "$ENCRYPT_KEY" ] || ENCRYPT_KEY="base64:<随机生成的 32 字节>"
  [ -n "$HASHIDS_SALT" ] || HASHIDS_SALT="<随机 32 位 hex>"
  [ -n "$JWT_SECRET" ] || JWT_SECRET="<随机 64 位 hex>"
  [ -n "$DB_PASS" ] || DB_PASS="<随机 24 位字母数字>"
  [ -n "$ADMIN_PASSWORD" ] || ADMIN_PASSWORD="<随机 16 位>"
  render_conf >&2
  printf '\n%s──── systemd 单元 %s.service ────%s\n' "$C_BLD" "$SERVICE_NAME" "$C_OFF" >&2
  render_unit | sed -e "s|@APP_NAME@|$(esc_repl "$APP_NAME")|g" \
    -e "s|@RUN_USER@|$(esc_repl "${RUN_USER:-$(id -un)}")|g" \
    -e "s|@RUN_GROUP@|$(esc_repl "${RUN_GROUP:-$(id -gn)}")|g" \
    -e "s|@PREFIX@|$(esc_repl "$PREFIX")|g" >&2
  printf '\n%s──── nginx 站点 %s ────%s\n' "$C_BLD" "$SERVICE_NAME" "$C_OFF" >&2
  render_vhost | sed -e "s|@HTTP_PORT@|$(esc_repl "$HTTP_PORT")|g" \
    -e "s|@BACKEND_PORT@|$(esc_repl "$BACKEND_PORT")|g" \
    -e "s|@PREFIX@|$(esc_repl "$PREFIX")|g" >&2
  printf '\n%s干跑结束，没有写任何文件。%s\n' "$C_GRN" "$C_OFF" >&2
  exit "$EXIT_OK"
fi
}

preflight() {
say "预检"

arch=$(uname -s)-$(uname -m)
[ "$(uname -s)" = Linux ] || step_fail "$EXIT_PREFLIGHT" "只支持 Linux（当前 $arch）"
[ "$(uname -m)" = x86_64 ] || step_fail "$EXIT_PREFLIGHT" \
  "只提供 x86_64 产物（当前 $arch）。ARM 请用 --mode build 从源码构建，或用 Docker"

missing=''
for c in curl tar openssl install sed awk grep ss sha256sum; do
  have "$c" || missing="$missing $c"
done
[ -z "$missing" ] || step_fail "$EXIT_PREFLIGHT" "缺少命令：$missing（装完再跑）"

if [ "$MODE" = build ]; then
  have cargo || step_fail "$EXIT_PREFLIGHT" "--mode build 需要 cargo（rust 1.99+）"
  have node  || step_fail "$EXIT_PREFLIGHT" "--mode build 需要 node（20+）"
  have pnpm  || step_fail "$EXIT_PREFLIGHT" "--mode build 需要 pnpm（9+）"
  [ -f "$REPO_DIR/Cargo.toml" ] || step_fail "$EXIT_PREFLIGHT" \
    "--repo 指的目录里没有 Cargo.toml：$REPO_DIR"
fi

# --keep-conf 是升级路径：下面第 2 步要靠已有 conf 里的 dsn 决定连哪个库，
# 没有 conf 就没得「保留」——在写盘之前拦住。
[ "$KEEP_CONF" = 0 ] || [ -f "$CONF" ] || step_fail "$EXIT_PREFLIGHT" \
  "--keep-conf 但 $CONF 不存在。它是升级用的（先正常装一次，再用它换二进制）；全新安装去掉这个参数"

if [ -e "$PREFIX" ] && [ ! -w "$PREFIX" ] && ! can_root; then
  step_fail "$EXIT_PREFLIGHT" "$PREFIX 不可写，且没有免密 sudo。
  换个地方：--prefix ~/bee-admin    或用 root 跑：sudo bash $PROG"
fi

for p in "$HTTP_PORT" "$BACKEND_PORT"; do
  if port_busy "$p"; then
    step_fail "$EXIT_PREFLIGHT" "端口 $p 已被占用。
  换端口：--http-port 8091 --backend-port 8090（或先停掉占用它的进程）"
  fi
done

if [ -n "$RUN_USER" ] && ! id -u "$RUN_USER" >/dev/null 2>&1; then
  step_fail "$EXIT_PREFLIGHT" "用户不存在：$RUN_USER"
fi
[ -n "${RUN_USER:-}" ] || RUN_USER=$( [ "$(id -u)" = 0 ] && echo "${SUDO_USER:-root}" || id -un )
[ -n "${RUN_GROUP:-}" ] || RUN_GROUP=$(id -gn "$RUN_USER" 2>/dev/null || id -gn)

if [ "$HTTP_PORT" -lt 1024 ] && ! can_root; then
  warn "端口 $HTTP_PORT < 1024，非 root 起不来 nginx；建议换个高位端口"
fi
info "架构 $arch · 用户 $RUN_USER:$RUN_GROUP · 端口 $HTTP_PORT → $BACKEND_PORT"
}

guard_conf() {
OLD_CONF_BACKUP=''
if [ -f "$CONF" ]; then
  old_key=$(ini_get "$CONF" app encrypt_key)
  old_fp=$(key_fingerprint "$old_key")
  if [ "$KEEP_CONF" = 1 ]; then
    say "复用已有配置（--keep-conf）"
    info "$CONF 保持原样不动（encrypt_key 指纹 $old_fp）"
    conf_addr=$(ini_get "$CONF" app http_addr)
    conf_upload=$(ini_get "$CONF" app upload_dir)
    # 只报告不修：配置是事实来源
    case ${conf_addr##*:} in
      "$BACKEND_PORT") ;;
      '') warn "已有配置里没有 [app] http_addr —— 单元渲染按 --backend-port $BACKEND_PORT 走" ;;
      *) warn "已有配置里 http_addr 是 $conf_addr，与 --backend-port $BACKEND_PORT 不一致；以配置为准，nginx 会反代到 ${conf_addr##*:}" ;;
    esac
    [ -n "$conf_upload" ] || warn "已有配置里没有 [app] upload_dir（默认 uploads）"
    case $old_key in
      ''|*change-me*) warn "已有配置的 encrypt_key 是空的/占位符 —— 服务**起不来**，请先修好再装" ;;
    esac
    conf_jwt=$(ini_get "$CONF" jwt secret)
    [ ${#conf_jwt} -ge 32 ] || warn "已有配置的 [jwt] secret 不足 32 字符 —— 服务起不来"
    # 库/账号也得按已有 DSN 走：保留的 conf 就是这个服务的连接参数，命令行上现生成的
    # 随机口令在这里毫无意义。**曾经没这么干**：拿新随机 DB_PASS 去连 → 连不上 →
    # 走建库建号 → ALTER USER 把库里的账号口令改掉，而 conf 里还是旧口令 ——
    # 所谓「升级」一次就把自己装坏，自检必然 Access denied。
    conf_dsn=$(ini_get "$CONF" db dsn)
    if [ -z "$conf_dsn" ]; then
      warn "已有配置里没有 [db] dsn —— 数据库检查退回命令行参数"
    elif [ "${conf_dsn#mysql://}" = "$conf_dsn" ]; then
      warn "已有配置里的 [db] dsn 不是 mysql:// 形式 —— 数据库检查退回命令行参数"
    else
      rest=${conf_dsn#mysql://}; userpass=${rest%@*}; hostpart=${rest#*@}
      DB_USER=$(urldec "${userpass%%:*}"); DB_PASS=$(urldec "${userpass#*:}")
      # 先切掉 /db 再切 host:port：`${hostpart##*:}` 会一路吃到路径里的冒号位置，
      # 把端口读成 "3306/bee_admin_install_test"（自检时逮到的）
      DB_NAME=${hostpart#*/}; DB_NAME=${DB_NAME%%\?*}
      hostport=${hostpart%%/*}
      DB_HOST=${hostport%%:*}; DB_PORT=${hostport##*:}
      case $hostport in *:*) ;; *) DB_PORT=3306 ;; esac
      info "数据库按已有配置：$DB_USER@$DB_HOST:$DB_PORT/$DB_NAME（--keep-conf 下 --db-* 不参与）"
    fi
  elif [ "$FORCE" = 1 ]; then
    backup="$CONF.bak.$(date -u '+%Y%m%d%H%M%S')"
    say "覆盖已有配置（--force）"
    cp -p "$CONF" "$backup"      # cp -p 保留权限/时间；**里面含原密钥，所以可恢复**
    OLD_CONF_BACKUP=$backup
    warn "已覆盖 $CONF，原文件备份到 $backup"
    warn "新生成的 encrypt_key 与旧的**不同**：库里已加密的 email/phone 将解不开；"
    warn "hashids_salt 也变了：已发出的对外短串全部作废。要回滚就用上面那个备份。"
  else
    printf '\n%s[中止]%s 已经装过（%s 存在），没有做任何修改。\n\n' "$C_YEL" "$C_OFF" "$CONF" >&2
    info "配置文件：$CONF"
    info "属主/权限：$(stat -c '%U:%G %a' "$CONF" 2>/dev/null || echo '?')   修改时间：$(stat -c '%y' "$CONF" 2>/dev/null | cut -d. -f1)"
    info "encrypt_key 指纹：$old_fp（只用于认出是哪把密钥，不是密钥本身）"
    cat >&2 <<EOF

  三选一：
    1) 升级（推荐）：只换二进制与服务文件，配置一个字节不动
         bash $PROG --keep-conf$(printf ' %s' "${ORIG_ARGS[@]+"${ORIG_ARGS[@]}"}")
    2) 重装配置（危险）：先备份再覆盖 —— encrypt_key 变了库里 email/phone 解不开
         bash $PROG --force ...
    3) 什么都不做（就是现在这样）

EOF
    exit "$EXIT_CONF_EXISTS"
  fi
fi
}

prepare_secrets() {
[ -n "$ENCRYPT_KEY" ]   || ENCRYPT_KEY="base64:$(openssl rand -base64 32 | tr -d '\n')"
[ -n "$HASHIDS_SALT" ]  || HASHIDS_SALT=$(openssl rand -hex 16)
[ -n "$JWT_SECRET" ]    || JWT_SECRET=$(openssl rand -hex 32)
DB_PASS_GENERATED=0
ADMIN_PASS_GENERATED=0
[ -n "$DB_PASS" ]       || { DB_PASS=$(gen_pass 24);     DB_PASS_GENERATED=1; }
[ -n "$ADMIN_PASSWORD" ]|| { ADMIN_PASSWORD=$(gen_pass_h 16); ADMIN_PASS_GENERATED=1; }

# 交互式补 DB 口令：只在「没给口令（刚才是随机的）」且「没 --yes」时问。
# 从 /dev/tty 读，所以 `curl | bash` 也问得出来（那时 stdin 是脚本本身）。
ask_db_pass=0
if [ "$KEEP_CONF" = 0 ] && [ "$DB_PASS_GENERATED" = 1 ] && [ "$ASSUME_YES" = 0 ] && [ -r /dev/tty ]; then
  ask_db_pass=1
fi
if [ "$ask_db_pass" = 1 ]; then
  printf '\n数据库 %s@%s:%s/%s 的口令（直接回车=用随机生成的）：' "$DB_USER" "$DB_HOST" "$DB_PORT" "$DB_NAME" >&2
  read -rs _p </dev/tty || _p=''
  printf '\n' >&2
  if [ -n "${_p:-}" ]; then DB_PASS=$_p; DB_PASS_GENERATED=0; fi
  unset _p
fi

if [ "$KEEP_CONF" = 0 ]; then
  # 注意：`&` 不能放进 [ ] 里（bash 5.2 解析 case 模式时会报语法错误），用 | 分支
  case $DB_PASS in *[/#?%]*|*"&"*|*"+"*) warn "数据库口令里含 / # ? % & + —— 已做 URL 编码写进 DSN，但别的工具里也可能咬人" ;; esac
fi
}
