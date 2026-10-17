# install.sh · 默认值 / 参数解析 / 校验 / 派生路径（由 install.sh source 后调用 parse_args "$@"）。
MODE=${BRA_MODE:-download}
VERSION=${BRA_VERSION:-latest}
BASE_URL=${BRA_BASE_URL:-}
PREFIX=${BRA_PREFIX:-/opt/bee-admin}
SERVICE_NAME=${BRA_SERVICE_NAME:-bee-admin}
RUN_USER=${BRA_USER:-}
RUN_GROUP=${BRA_GROUP:-}
REPO_DIR=${BRA_REPO:-}
HTTP_PORT=${BRA_HTTP_PORT:-8081}
BACKEND_PORT=${BRA_BACKEND_PORT:-8080}
DB_HOST=${BRA_DB_HOST:-127.0.0.1}
DB_PORT=${BRA_DB_PORT:-3306}
DB_NAME=${BRA_DB_NAME:-bee_admin}
DB_USER=${BRA_DB_USER:-bee_admin}
DB_PASS=${BRA_DB_PASS:-}
DB_ADMIN_USER=${BRA_DB_ADMIN_USER:-}
DB_ADMIN_PASS=${BRA_DB_ADMIN_PASS:-}
ENCRYPT_KEY=${BRA_ENCRYPT_KEY:-}
JWT_SECRET=${BRA_JWT_SECRET:-}
HASHIDS_SALT=${BRA_HASHIDS_SALT:-}
SNOWFLAKE=${BRA_SNOWFLAKE:-0,0}
ADMIN_PASSWORD=${BRA_ADMIN_PASSWORD:-}
APP_NAME=${BRA_APP_NAME:-bee-rust-admin}
KEEP_CONF=0; FORCE=0; WITH_SYSTEMD=1; WITH_NGINX=1; DRY_RUN=0; ASSUME_YES=0
NGINX_CONF_DIR=${BRA_NGINX_CONF_DIR:-}

need_val() { [ "$2" -ge 1 ] || die "$EXIT_FAIL" "$1 需要一个值（--help 看用法）"; }

parse_args() {
while [ $# -gt 0 ]; do
  case $1 in
    --mode)            need_val "$1" $#; MODE=$2; shift 2;;
    --version)         need_val "$1" $#; VERSION=$2; shift 2;;
    --base-url)        need_val "$1" $#; BASE_URL=$2; shift 2;;
    --prefix)          need_val "$1" $#; PREFIX=$2; shift 2;;
    --service-name)    need_val "$1" $#; SERVICE_NAME=$2; shift 2;;
    --user)            need_val "$1" $#; RUN_USER=$2; shift 2;;
    --group)           need_val "$1" $#; RUN_GROUP=$2; shift 2;;
    --repo)            need_val "$1" $#; REPO_DIR=$2; shift 2;;
    --http-port)       need_val "$1" $#; HTTP_PORT=$2; shift 2;;
    --backend-port)    need_val "$1" $#; BACKEND_PORT=$2; shift 2;;
    --db-host)         need_val "$1" $#; DB_HOST=$2; shift 2;;
    --db-port)         need_val "$1" $#; DB_PORT=$2; shift 2;;
    --db-name)         need_val "$1" $#; DB_NAME=$2; shift 2;;
    --db-user)         need_val "$1" $#; DB_USER=$2; shift 2;;
    --db-pass)         need_val "$1" $#; DB_PASS=$2; shift 2;;
    --db-admin-user)   need_val "$1" $#; DB_ADMIN_USER=$2; shift 2;;
    --db-admin-pass)   need_val "$1" $#; DB_ADMIN_PASS=$2; shift 2;;
    --encrypt-key)     need_val "$1" $#; ENCRYPT_KEY=$2; shift 2;;
    --jwt-secret)      need_val "$1" $#; JWT_SECRET=$2; shift 2;;
    --hashids-salt)    need_val "$1" $#; HASHIDS_SALT=$2; shift 2;;
    --snowflake)       need_val "$1" $#; SNOWFLAKE=$2; shift 2;;
    --admin-password)  need_val "$1" $#; ADMIN_PASSWORD=$2; shift 2;;
    --app-name)        need_val "$1" $#; APP_NAME=$2; shift 2;;
    --nginx-conf-dir)  need_val "$1" $#; NGINX_CONF_DIR=$2; shift 2;;
    --keep-conf)       KEEP_CONF=1; shift;;
    --force)           FORCE=1; shift;;
    --no-systemd)      WITH_SYSTEMD=0; shift;;
    --no-nginx)        WITH_NGINX=0; shift;;
    --dry-run)         DRY_RUN=1; shift;;
    --yes|-y)          ASSUME_YES=1; shift;;
    --help|-h)         usage; exit "$EXIT_OK";;
    -*)                die "$EXIT_FAIL" "未知参数：$1（--help 看用法）";;
    *)                 die "$EXIT_FAIL" "多余的位置参数：$1（--help 看用法）";;
  esac
done

case $MODE in download|build) ;; *) die "$EXIT_FAIL" "--mode 只能是 download 或 build，给的是 $MODE";; esac
case $SNOWFLAKE in [0-9]*,[0-9]*) ;; *) die "$EXIT_FAIL" "--snowflake 形如 0,0（节点号,数据中心号）";; esac
case $DB_NAME in *[!A-Za-z0-9_]*) die "$EXIT_FAIL" "--db-name 只允许字母数字下划线（它要拼进 SQL）";; esac
for p in "$HTTP_PORT" "$BACKEND_PORT" "$DB_PORT"; do
  case $p in *[!0-9]*|'') die "$EXIT_FAIL" "端口必须是数字，给的是：$p";; esac
done

# 仓库根：脚本所在目录（curl | bash 时没有仓库；--repo 可显式给）
[ -z "$REPO_DIR" ] && REPO_DIR=$SCRIPT_DIR

PREFIX=${PREFIX%/}
CONF=$PREFIX/conf/app.conf
STAGE_ETC=$PREFIX/etc
UNIT_FILE=$STAGE_ETC/$SERVICE_NAME.service
VHOST_FILE=$STAGE_ETC/nginx-$SERVICE_NAME.conf
VERSION_FILE=$PREFIX/VERSION

GITHUB_REPO=erikwang2013/bee-rust-admin
[ -z "$BASE_URL" ] && BASE_URL="https://github.com/$GITHUB_REPO/releases/download"
}
