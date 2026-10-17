# install.sh · 启动、自检与收尾。
start_and_selfcheck() {
manual_start=0
if [ "$WITH_SYSTEMD" != 1 ]; then
  # --no-systemd：单元文件根本没装进系统，`restart $SERVICE_NAME` 只会打到**同名的别的单元**
  # （默认服务名就是 bee-admin —— 那多半是生产那个），所以这里坚决不碰 systemd。
  # 手动启动命令在收尾处打印（与 need-root 的提示同一口径）。
  say "跳过重启服务（--no-systemd，单元没有装）"
  manual_start=1
elif run_root systemctl restart "$SERVICE_NAME" 2>/dev/null; then
  info "已重启 $SERVICE_NAME.service"
  sleep 2
else
  manual_start=1
fi

say "自检"
backend_ok=0
if [ "$manual_start" = 0 ]; then
  # systemd 托管：直接探后端端口。
  # 注意**不用登录探针**——默认开着验证码，登录必然 400，测不了；
  # 而健康检查能回 OK 就说明配置解析、DSN、建表、种子全都过去了（任一步失败进程会直接退出）。
  if selfcheck_http "http://127.0.0.1:$listen_port/api/v1/health" OK "后端"; then
    backend_ok=1
    info "后端健康检查通过（127.0.0.1:$listen_port）"
  else
    printf '\n%s[自检失败]%s 后端没起来。看这几样：\n' "$C_RED" "$C_OFF" >&2
    printf '    systemctl status %s -l --no-pager\n    journalctl -u %s -n 100 --no-pager\n' "$SERVICE_NAME" "$SERVICE_NAME" >&2
    printf '    cat %s   # 检查 dsn / encrypt_key / jwt secret（**别贴给别人看**）\n' "$CONF" >&2
    exit "$EXIT_SELFTEST"
  fi
else
  # 没托管：前台起一个进程探完就停（--no-systemd，或 systemd 不可用/没 root）
  say "后端未托管，先直接起一个前台进程探一下"
  BEE_ADMIN_CONF=$CONF BEE_ADMIN_HTTP_ADDR="127.0.0.1:$listen_port" \
    "$PREFIX/bin/bee_admin" > "$TMP/backend.log" 2>&1 &
  probe_pid=$!
  if selfcheck_http "http://127.0.0.1:$listen_port/api/v1/health" OK "后端"; then
    backend_ok=1
    info "后端健康检查通过（进程 $probe_pid，探完就停）"
  else
    printf '\n%s[自检失败]%s 后端没起来。它最后说的话：\n' "$C_RED" "$C_OFF" >&2
    tail -20 "$TMP/backend.log" | sed 's/^/    /' >&2
    cat >&2 <<EOF

  排障：
    cat $CONF              # 检查 dsn / encrypt_key / jwt secret（**别贴给别人看**：里面有密钥和口令）
    手动前台起一次看日志：
      BEE_ADMIN_CONF=$CONF $PREFIX/bin/bee_admin
    渲染好的服务文件在 $UNIT_FILE 与 $VHOST_FILE
EOF
  fi
  kill "$probe_pid" 2>/dev/null || true
  wait "$probe_pid" 2>/dev/null || true
  [ "$backend_ok" = 1 ] || exit "$EXIT_SELFTEST"
fi
}

selfcheck_http() { # $1=url $2=期望内容
  local i body
  for i in $(seq 1 15); do
    body=$(curl -fsS --max-time 5 "$1" 2>/dev/null || true)
    case $body in *"$2"*) return 0 ;; esac
    sleep 1
  done
  return 1
}

finish() {
printf '\n%s════════ 装好了 ════════%s\n' "$C_GRN$C_BLD" "$C_OFF" >&2
info "安装根：$PREFIX    版本：$(cat "$VERSION_FILE" 2>/dev/null || echo '?')"
[ "$manual_start" = 1 ] && {
  if [ -n "$need_root_cmds" ]; then
    printf '\n%s服务还没有被 systemd 托管。执行下面这些就完事（需要 root）：%s\n' "$C_YEL" "$C_OFF" >&2
    printf '%s\n' "$need_root_cmds" >&2
    printf '\n' >&2
  else
    # 走到这里 = 服务没被 systemd 托管（--no-systemd、--no-nginx、或没 root）。
    # 只陈述这一件事：站点装没装是另一码事，别一起说。
    printf '\n%s服务没有被 systemd 托管（--no-systemd / --no-nginx / 没 root）：单元只渲染到 %s/etc/，没装进系统。%s\n' \
      "$C_YEL" "$PREFIX" "$C_OFF" >&2
    printf '  要这么起（前台）：\n' >&2
  fi
  printf '    BEE_ADMIN_CONF=%s %s/bin/bee_admin\n' "$CONF" "$PREFIX" >&2
}

# 整条链路（nginx → 后端）：只有在「root 都装上了、nginx 也 reload 过」时才有意义，
# 否则 nginx 还没加载新站点，测了只会吓人
if [ "$backend_ok" = 1 ] && [ "$manual_start" = 0 ] && [ "$WITH_NGINX" = 1 ] && [ -z "$need_root_cmds" ]; then
  if selfcheck_http "http://127.0.0.1:$HTTP_PORT/api/v1/health" OK "链路"; then
    info "链路自检通过（nginx → 后端）"
  else
    warn "nginx 链路没通（http://127.0.0.1:$HTTP_PORT/api/v1/health）—— 看 nginx -t 与后端日志"
  fi
fi

cat >&2 <<EOF

  打开：  http://<这台机器的地址>:$HTTP_PORT
  账号：  admin
EOF
if [ "$ADMIN_PASS_GENERATED" = 1 ]; then
  printf '  初始口令：%s%s%s   ← 只显示这一次，登录后立刻改\n' "$C_BLD" "$ADMIN_PASSWORD" "$C_OFF" >&2
else
  printf '  初始口令：你通过 --admin-password 指定的那个\n' >&2
fi
[ "$DB_PASS_GENERATED" = 1 ] && \
  printf '  数据库口令（已写进 conf）：%s%s%s\n' "$C_YEL" "$DB_PASS" "$C_OFF" >&2
cat >&2 <<EOF

  说明：
    · 初始口令**只在库为空（首次启动）时生效**；库已有 admin 的话它被忽略，请登录后改密
    · ⚠️ $CONF 里的 encrypt_key **不可更换**（换了库里 email/phone 解不开），备份它
    · 升级再来一次：bash $PROG --keep-conf ...（配置一个字节不动）
    · 回滚二进制：mv $PREFIX/bin/bee_admin.prev $PREFIX/bin/bee_admin && systemctl restart $SERVICE_NAME
    · 卸载：systemctl disable --now $SERVICE_NAME，再删 $PREFIX 与上面的服务文件（数据库自己决定要不要删）
EOF
[ -n "$OLD_CONF_BACKUP" ] && printf '    · 旧配置备份：%s\n' "$OLD_CONF_BACKUP" >&2
[ -n "$OLD_CONF_BACKUP" ] && warn "这次覆盖了配置：库里已加密的 email/phone 用新的 encrypt_key 解不开。"
printf '\n' >&2
}
