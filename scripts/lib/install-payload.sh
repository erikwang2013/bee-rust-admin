# install.sh · 落产物与数据库就绪检查。
install_payload() {
say "安装到 $PREFIX"
mkdir -p "$PREFIX/bin" "$PREFIX/conf" "$PREFIX/web" "$PREFIX/uploads" "$STAGE_ETC" "$TMP"

if [ -f "$BIN_NEW" ]; then
  install -m0755 "$BIN_NEW" "$TMP/bee_admin.staged"
  [ -f "$PREFIX/bin/bee_admin" ] && mv -f "$PREFIX/bin/bee_admin" "$PREFIX/bin/bee_admin.prev"
  mv -f "$TMP/bee_admin.staged" "$PREFIX/bin/bee_admin"
  info "二进制 → $PREFIX/bin/bee_admin$([ -f "$PREFIX/bin/bee_admin.prev" ] && echo "（旧版留作 bee_admin.prev，回滚就一条 mv）")"
fi

if [ -f "$TMP/bee-admin-web-dist.tar.gz" ]; then
  rm -rf "$DIST_NEW"; mkdir -p "$DIST_NEW"
  tar xzf "$TMP/bee-admin-web-dist.tar.gz" -C "$DIST_NEW"
  [ -f "$DIST_NEW/index.html" ] || step_fail "$EXIT_FAIL" "前端产物里没有 index.html（压缩包内容不对）"
  rm -rf "$PREFIX/web/dist.old"
  [ -d "$PREFIX/web/dist" ] && mv "$PREFIX/web/dist" "$PREFIX/web/dist.old"
  mv "$DIST_NEW" "$PREFIX/web/dist"
  rm -rf "$PREFIX/web/dist.old"
  chmod -R a+rX "$PREFIX/web/dist"     # nginx 的 worker 用户（www/www-data）要能读
  info "前端 → $PREFIX/web/dist"
fi

if [ "$KEEP_CONF" = 0 ]; then
  render_conf > "$TMP/app.conf.new"
  install -m0600 "$TMP/app.conf.new" "$CONF"
  info "配置 → $CONF（0600）"
fi
printf '%s\n' "${VERSION_FILE_VALUE:-$VERSION}" > "$VERSION_FILE"
}

prepare_database() {
say "检查数据库"
db_ready=0
if have mysql; then
  # 先试应用账号（库/号已存在的老机器走这条）
  if MYSQL_PWD=$DB_PASS mysql -h"$DB_HOST" -P"$DB_PORT" -u"$DB_USER" -D"$DB_NAME" \
       -e 'SELECT 1' >/dev/null 2>&1; then
    info "应用账号可连（$DB_USER@$DB_HOST:$DB_PORT/$DB_NAME）"
    db_ready=1
  elif [ -n "$DB_ADMIN_USER" ] && [ "$KEEP_CONF" = 1 ]; then
    # 保留的配置里是旧口令，ALTER USER 只会把库和配置改成对不上
    warn "--keep-conf：不建库建号（配置里的 DSN 是事实来源）。先按上面的提示修好 dsn 再重跑"
  elif [ -n "$DB_ADMIN_USER" ]; then
    say "用管理账号建库建号"
    grant_host=$DB_HOST
    case $grant_host in 127.0.0.1|localhost) grant_host=127.0.0.1 ;; esac
    sql=$(cat <<EOF
CREATE DATABASE IF NOT EXISTS \`$DB_NAME\` CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;
CREATE USER IF NOT EXISTS '$DB_USER'@'$grant_host' IDENTIFIED BY '$(printf '%s' "$DB_PASS" | sed "s/'/''/g")';
ALTER USER '$DB_USER'@'$grant_host' IDENTIFIED BY '$(printf '%s' "$DB_PASS" | sed "s/'/''/g")';
GRANT ALL PRIVILEGES ON \`$DB_NAME\`.* TO '$DB_USER'@'$grant_host';
FLUSH PRIVILEGES;
EOF
)
    info "将执行："
    printf '%s\n' "$sql" | sed 's/^/    /' >&2
    if printf '%s\n' "$sql" | MYSQL_PWD=$DB_ADMIN_PASS mysql -h"$DB_HOST" -P"$DB_PORT" \
         -u"$DB_ADMIN_USER" >/dev/null 2>&1; then
      if MYSQL_PWD=$DB_PASS mysql -h"$DB_HOST" -P"$DB_PORT" -u"$DB_USER" -D"$DB_NAME" \
           -e 'SELECT 1' >/dev/null 2>&1; then
        info "建好了，应用账号连得上"
        db_ready=1
      else
        warn "建库执行了，但应用账号仍连不上（授权主机名？口令？）"
      fi
    else
      warn "管理账号执行失败（权限不足或口令不对）"
    fi
  fi
else
  warn "本机没有 mysql 客户端，跳过连接测试"
fi

if [ "$db_ready" = 0 ]; then
  cat >&2 <<EOF

${C_YEL}需要你先建好数据库${C_OFF}（脚本不 DROP、不猜 root 口令、不装 MySQL）。
用有 CREATE USER 权限的账号执行（口令换成你自己的）：

  CREATE DATABASE IF NOT EXISTS \`$DB_NAME\` CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;
  CREATE USER IF NOT EXISTS '$DB_USER'@'127.0.0.1' IDENTIFIED BY '你的口令';
  GRANT ALL PRIVILEGES ON \`$DB_NAME\`.* TO '$DB_USER'@'127.0.0.1';
  FLUSH PRIVILEGES;

建好之后重跑本脚本（或加 --db-admin-user/--db-admin-pass 让它自动建）。
表结构与超管账号由服务**首次启动时自动创建**，不需要手工导入 SQL。

EOF
  step_fail "$EXIT_NEED_DB" "数据库还没就绪"
fi
}
