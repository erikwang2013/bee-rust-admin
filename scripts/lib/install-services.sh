# install.sh · 渲染并安装 systemd 单元 / nginx 站点。
install_services() {
need_root_cmds=''

render_unit | sed -e "s|@APP_NAME@|$(esc_repl "$APP_NAME")|g" \
  -e "s|@RUN_USER@|$(esc_repl "$RUN_USER")|g" \
  -e "s|@RUN_GROUP@|$(esc_repl "$RUN_GROUP")|g" \
  -e "s|@PREFIX@|$(esc_repl "$PREFIX")|g" > "$TMP/unit.rendered"
render_vhost | sed -e "s|@HTTP_PORT@|$(esc_repl "$HTTP_PORT")|g" \
  -e "s|@BACKEND_PORT@|$(esc_repl "$BACKEND_PORT")|g" \
  -e "s|@PREFIX@|$(esc_repl "$PREFIX")|g" > "$TMP/vhost.rendered"

install -m0644 "$TMP/unit.rendered" "$UNIT_FILE"
install -m0644 "$TMP/vhost.rendered" "$VHOST_FILE"
info "已渲染：$UNIT_FILE"
info "已渲染：$VHOST_FILE"

# 后端实际监听地址：--keep-conf 时以配置为准
listen_addr=$(ini_get "$CONF" app http_addr 2>/dev/null || true)
listen_port=${listen_addr##*:}; [ -n "$listen_port" ] || listen_port=$BACKEND_PORT

if [ "$WITH_SYSTEMD" = 1 ]; then
  say "安装 systemd 单元"
  if run_root install -m0644 "$UNIT_FILE" "/etc/systemd/system/$SERVICE_NAME.service"; then
    run_root systemctl daemon-reload || true
    run_root systemctl enable "$SERVICE_NAME" >/dev/null 2>&1 || true
    info "已安装 /etc/systemd/system/$SERVICE_NAME.service"
  else
    need_root_cmds="$need_root_cmds
  sudo install -m0644 '$UNIT_FILE' '/etc/systemd/system/$SERVICE_NAME.service'
  sudo systemctl daemon-reload && sudo systemctl enable --now $SERVICE_NAME"
    warn "没有 root（或免密 sudo 不可用），systemd 单元只渲染到 $UNIT_FILE"
  fi
fi

if [ "$WITH_NGINX" = 1 ]; then
  say "安装 nginx 站点"
  ndir=$NGINX_CONF_DIR
  if [ -z "$ndir" ] && have nginx; then
    # 自编译版（LNMP）的站点目录在 --prefix 下，且**只有被 nginx.conf include 的才算**；
    # 发行版在 /etc/nginx 下。都不认就宁可只暂存 + 打印命令，也不猜着写。
    npfx=$(nginx -V 2>&1 | tr ' ' '\n' | sed -n 's/^--prefix=//p' | tail -1)
    if [ -n "$npfx" ] && [ -f "$npfx/conf/nginx.conf" ]; then
      for sub in conf/vhost conf/conf.d; do
        cand="$npfx/$sub"
        if [ -d "$cand" ] && grep -qE "include[[:space:]]+.*$(basename "$cand")" "$npfx/conf/nginx.conf" 2>/dev/null; then
          ndir=$cand; break
        fi
      done
    fi
    if [ -z "$ndir" ] && [ -d /etc/nginx/conf.d ] \
       && grep -qE 'include[[:space:]]+.*conf\.d' /etc/nginx/nginx.conf 2>/dev/null; then
      ndir=/etc/nginx/conf.d
    fi
    if [ -z "$ndir" ] && [ -d /etc/nginx/sites-enabled ]; then ndir=/etc/nginx/sites-enabled; fi
  fi
  target="${ndir:-}/$SERVICE_NAME.conf"
  if [ -n "$ndir" ] && [ -e "$target" ] && ! cmp -s "$VHOST_FILE" "$target" && [ "$FORCE" != 1 ]; then
    warn "$target 已存在且内容不同 —— 没有覆盖（加 --force 才覆盖，或删掉那个文件）"
    ndir=''
  fi
  if [ -n "$ndir" ] && run_root install -m0644 "$VHOST_FILE" "$target"; then
    if run_root nginx -t >/dev/null 2>&1; then
      if have systemctl && systemctl is-active nginx >/dev/null 2>&1; then
        run_root systemctl reload nginx || warn "reload nginx 失败，看 nginx -t 的输出"
      else
        run_root nginx -s reload || warn "nginx -s reload 失败（自编译版可能没有 systemd 单元）"
      fi
      info "已安装并 reload：$target"
    else
      run_root rm -f "$target"
      warn "nginx -t 不通过，已把 $target 撤掉。检查：nginx -t"
    fi
  else
    need_root_cmds="$need_root_cmds
  sudo install -m0644 '$VHOST_FILE' '${ndir:-/etc/nginx/conf.d}/$SERVICE_NAME.conf'
  sudo nginx -t && sudo systemctl reload nginx"
    warn "没有 root（或没探到 nginx 站点目录），vhost 只渲染到 $VHOST_FILE"
  fi
fi
}
