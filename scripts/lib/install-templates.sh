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
    }

    location ~* \.(js|css|woff2?|svg|png|jpg|ico)$ {
        expires 30d;
        add_header Cache-Control "public, immutable";
    }
    location = /index.html {
        add_header Cache-Control "no-cache";
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
