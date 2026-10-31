# install.sh · 模板渲染（与 admin/deploy/*.example 同源；改了那边记得同步这里）。
render_unit() {
  cat <<'EOF'
[Unit]
Description=bee-rust admin backend (@APP_NAME@)
After=network.target mysql.service
Wants=mysql.service

[Service]
Type=simple
User=@RUN_USER@
Group=@RUN_GROUP@
WorkingDirectory=@PREFIX@
Environment=BEE_ADMIN_CONF=@PREFIX@/conf/app.conf
ExecStart=@PREFIX@/bin/bee_admin
Restart=on-failure
RestartSec=3
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF
}

render_vhost() {
  cat <<'EOF'
# BRA 管理后台 —— nginx 站点（由 install.sh 渲染，改这里要同步 admin/deploy/nginx.conf.example）
server {
    listen @HTTP_PORT@;
    server_name _;

    root @PREFIX@/web/dist;
    index index.html;

    # CSV 导入走后端 /api/v1/imports；管理端请求体上限已配成 8 MiB，nginx 默认只有 1m，
    # 会先回 413。两处保持一致（multipart 边界等留余量），不是又放宽了业务上限。
    client_max_body_size 8m;

    # 点击劫持 / MIME 嗅探防护。注意 add_header 继承规则：下面任何自带 add_header 的
    # location 都不再继承这三行，必须自己重复一遍。
    add_header X-Frame-Options "DENY" always;
    add_header Content-Security-Policy "frame-ancestors 'none'" always;
    add_header X-Content-Type-Options "nosniff" always;

    gzip on;
    gzip_types text/css text/javascript application/javascript application/json image/svg+xml;
    gzip_min_length 1024;
    gzip_vary on;

    location / {
        try_files $uri $uri/ /index.html;
    }

    location /api/ {
        # 不带尾斜杠：原样转发完整 URI（/api/v1/auth/login 保持 /api/v1/auth/login）
        proxy_pass http://127.0.0.1:@BACKEND_PORT@;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        # 导入/导出是长耗时请求：前端给 /api/v1/imports 120s、后端放宽到 180s，
        # nginx 默认 proxy_read_timeout 60s 会先断开，这里按 180s 留足余量。
        proxy_read_timeout 180s;
        proxy_send_timeout 180s;
        # 大导出/附件不落响应缓冲：Compose 里 /var/cache/nginx 是 32MB tmpfs，
        # 缓冲写满 proxy_temp 会直接失败；只关响应缓冲，请求缓冲保持默认。
        proxy_buffering off;
    }

    # （两处都有自己的 add_header，所以安全头必须重复，否则不再继承 server 级）
    location ~* \.(js|css|woff2?|svg|png|jpg|ico)$ {
        expires 30d;
        add_header Cache-Control "public, immutable";
        add_header X-Frame-Options "DENY" always;
        add_header Content-Security-Policy "frame-ancestors 'none'" always;
        add_header X-Content-Type-Options "nosniff" always;
    }
    location = /index.html {
        add_header Cache-Control "no-cache";
        add_header X-Frame-Options "DENY" always;
        add_header Content-Security-Policy "frame-ancestors 'none'" always;
        add_header X-Content-Type-Options "nosniff" always;
    }
}
EOF
}

render_conf() {
  cat <<EOF
# 由 install.sh 生成于 $(date -u '+%Y-%m-%d %H:%M:%S UTC')
#
# ⚠️ encrypt_key 与 hashids_salt **不可更换**：
#    换 encrypt_key = 库里已加密的 email/phone 全部解不开（没有救援）；
#    换 hashids_salt = 所有已发出的对外短串作废。改前先备份这个文件。
[app]
name = $APP_NAME
http_addr = 127.0.0.1:$BACKEND_PORT
log_level = info
upload_dir = uploads
hashids_salt = $HASHIDS_SALT
hashids_min_len = 8
snowflake_worker = ${SNOWFLAKE%,*}
snowflake_dc = ${SNOWFLAKE#*,}
encrypt_key = $ENCRYPT_KEY

[db]
dsn = mysql://$(urlenc "$DB_USER"):$(urlenc "$DB_PASS")@$DB_HOST:$DB_PORT/$DB_NAME

[jwt]
secret = $JWT_SECRET
expire_hours = 24

[auth]
max_fail = 5
lock_minutes = 10
captcha = true

[log]
retain_days = 90

[job]
enabled = true

[security]
scan = off

[apidoc]
enabled = false
password =

[seed]
initial_admin_password = $ADMIN_PASSWORD
EOF
}
