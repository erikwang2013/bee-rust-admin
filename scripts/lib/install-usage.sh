# install.sh · --help 文本（原样保留；改参数时同步这里）。
usage() {
  cat <<'EOF'
BRA 一键安装脚本

用法：bash install.sh [参数]

安装方式
  --mode download|build   产物来源。download（默认）= 拉 GitHub Releases 的预编译产物；
                          build = 从源码构建（需要 rust 1.99+ / node 20+ / pnpm 9+）
  --version TAG           要装的版本（download 模式）。默认 latest（查 GitHub）

位置与命名
  --prefix DIR            安装根目录（默认 /opt/bee-admin）
  --service-name NAME     systemd 单元名 / nginx vhost 文件名（默认 bee-admin）
                          装第二套实例时靠它 + --prefix + 端口区分
  --user U / --group G    服务以什么身份跑（默认：root 时为 $SUDO_USER，否则当前用户）
  --repo DIR              源码目录（仅 --mode build；默认取脚本所在目录）
  --base-url URL          产物下载地址前缀，支持 file://（内网镜像 / 离线验证用）
                          默认 https://github.com/erikwang2013/bee-rust-admin/releases/download/<TAG>

端口
  --http-port N           对外端口（nginx listen，默认 8081）
  --backend-port N        后端监听端口（默认 8080，只被 nginx 反代，不对外）

数据库
  --db-host H             默认 127.0.0.1
  --db-port N             默认 3306
  --db-name N             默认 bee_admin
  --db-user U             默认 bee_admin（应用账号，不是 root）
  --db-pass P             应用账号口令。不给则随机生成并打印
  --db-admin-user U       给了才**自动建库建号**（需要 CREATE USER 权限的账号）
  --db-admin-pass P       同上。不给的话脚本只打印 SQL 让你手工执行

密钥与账号（不给就随机生成；从旧库迁移时可显式对齐）
  --encrypt-key K         [app] encrypt_key，格式 base64:<32字节>
                          ⚠️ 不可更换：换了库里已加密的 email/phone 全部解不开
  --jwt-secret S          [jwt] secret，≥32 字符
  --hashids-salt S        [app] hashids_salt，换了已发出的对外短串全部作废
  --snowflake W,D         雪花节点号,数据中心号（默认 0,0；多实例必须各不相同）
  --admin-password P      超管初始口令，默认随机。**只在库为空（首次启动）时生效**
  --app-name N            [app] name（默认 bee-rust-admin）

已有安装 / 危险操作
  --keep-conf             复用已有的 app.conf（升级路径）：只换二进制与配置好的服务文件，
                          配置文件一个字节都不动
  --force                 允许覆盖已有 app.conf。**会先备份**，但换了 encrypt_key 之后
                          库里数据仍可能解不开 —— 只有明确知道后果才用它
  --nginx-conf-dir DIR    显式指定 nginx 站点目录（默认自动探测）

其它
  --no-systemd            不装 systemd 单元，只渲染到 <prefix>/etc/ 并打印要执行的命令
  --no-nginx              同上（不装 nginx 站点）
  --dry-run               只打印将要写的一切，不落盘、不下载、不连库
  --yes, -y               不交互，全部用默认值/参数
  --help                  显示本帮助

示例
  # 最省事：交互式装一套（会问数据库口令）
  bash install.sh

  # 老机器（glibc < 2.38）从源码构建
  bash install.sh --mode build

  # 装第二套实例（端口与目录都错开）
  bash install.sh --service-name bee-admin-2 --prefix /opt/bee-admin-2 \
                  --http-port 8091 --backend-port 8090 --snowflake 2,0

  # 先看会做什么，什么都不写
  bash install.sh --dry-run --yes
EOF
}
