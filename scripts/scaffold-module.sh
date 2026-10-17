#!/usr/bin/env bash
# 模块脚手架（扩展工程项 10）：一条命令从一个模块名 + 权限前缀 + 字段清单生成模块骨架。
#
#   scripts/scaffold-module.sh <模块名> <权限前缀> [字段...] [选项]
#   scripts/scaffold-module.sh message system:message name:string:64 status:bool sort:int remark:text
#
# 字段写法 `<名>:<类型>[:<长度>]`，类型：string（VARCHAR，默认 128）/ text（TEXT）/
# int（i32）/ i64 / bool（i8，1 是 / 0 否）/ datetime（DATETIME，秒精度；接口收发都是
# "YYYY-MM-DD HH:MM:SS" 字符串，见 docs/scaffold.md）。**第一个 string 字段被当成名称字段**：
# 列表的模糊查询、新建的必填与超长校验都只挂在它身上。
#
# 生成物（骨架，不是成品）：模型 / 接口（4 个 CRUD 接口 + apidoc 注解）/ 单元测试 /
# 集成测试 / 前端 api 封装 / 前端页面。剩下的「真实业务逻辑」自己往里头长。
#
# 三条硬规则：
# 1. 生成物**必须自己先编译过**：跑完原地执行 `cargo check -p bee_admin --all-targets` +
#    `pnpm -C admin/web exec tsc -b`；编译不过 = 脚本的 bug，修脚本别改生成物糊过去
#    （`--out` 跳过自检：那只是把骨架写到仓库外对照用，不参与编译）。**任何一步失败都整体回滚**
#    （两个 mod.rs 还原、本次写下的文件删掉）：自检不过、写盘 I/O 错（磁盘满 / 权限 / 目标是
#    坏链接）、Ctrl-C 全走同一条路（EXIT 兜底），工作树回到跑脚本之前。
# 2. **不覆盖任何已有文件**：任一目标已存在就整体退出（一次误跑会清掉线上模块）。
# 3. 只追加 `api/v1/mod.rs` 一行 `pub mod`、`models/mod.rs` 的 `pub mod` + `pub use`
#    （不加，生成物不参与编译，自检就是假绿）；接口模块落 `admin/src/api/v1/<模块>/mod.rs`
#    （与 v1 的「目录 + mod.rs」约定一致）。路由/菜单种子/前端路由/i18n 只**打印可粘贴片段**
#    —— 结构化编辑脚本改错比不改更糟，位置只有人知道。
#
# 想先看骨架长什么样：`--out /tmp/sk` 重造既有模块做对照（如用字典管理的字段清单重造 dict）。
set -euo pipefail

die() { echo "scaffold: $*" >&2; exit 1; }

# 模板 / 接线清单 / 字段展开在 scripts/scaffold-templates.sh / scaffold-wiring.sh /
# scaffold-fields.sh（同 shell source 进来，heredoc 与变量语义一样；拆开只为守住
# 「文件 <500 行」的家规）。$ 展开与反引号转义在那边照旧。
SCRIPT_DIR=$(cd -- "$(dirname -- "$0")" 2>/dev/null && pwd -P) || die "定位不到脚本所在目录"
for lib in scaffold-templates.sh scaffold-wiring.sh scaffold-fields.sh; do
    [ -f "$SCRIPT_DIR/$lib" ] || die "缺脚本依赖：$SCRIPT_DIR/$lib"
    . "$SCRIPT_DIR/$lib"
done

usage() {
    # 打印头部说明到「三条硬规则」为止：按标题动态取，别写死行号（改注释就得多改一处）
    sed -n '2,/^# 三条硬规则/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
}

if [ $# -lt 2 ]; then
    if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then usage; exit 0; fi
    usage >&2
    exit 1
fi

MODULE=$1
PERM=$2
shift 2

FIELDS=()
LABEL=""
PAGE=""
API_PATH=""
OUT=""
while [ $# -gt 0 ]; do
    case "$1" in
        --label) LABEL=${2:-}; shift 2 ;;
        --path) PAGE=${2:-}; shift 2 ;;
        --api-path) API_PATH=${2:-}; shift 2 ;;
        --out) OUT=${2:-}; shift 2 ;;
        -h | --help) usage; exit 0 ;;
        --*) die "不认识的选项：$1" ;;
        *) FIELDS+=("$1"); shift ;;
    esac
done

# ── 参数校验（在写任何东西之前全查完）──────────────────────────
[[ $MODULE =~ ^[a-z][a-z0-9_]*$ ]] || die "模块名只能是小写字母/数字/下划线且字母开头：$MODULE"
case "$MODULE" in
    self | super | crate | mod | fn | let | type | use | ref | move | impl | trait | where \
        | async | await | struct | enum | const | static | match | loop | break | continue \
        | return | if | else | for | while | in | as | pub | mut | dyn | box | test)
        die "模块名不能是 Rust 关键字：$MODULE" ;;
esac
[[ $PERM =~ ^[a-z][a-z0-9_]*:[a-z][a-z0-9_]*$ ]] || die "权限前缀形如 system:message：$PERM"
[ ${#FIELDS[@]} -gt 0 ] || die "至少要有一个字段，例如 name:string:64"
LABEL=${LABEL:-$MODULE}
PAGE=${PAGE:-/system/$MODULE}
[[ $PAGE =~ ^/ ]] || die "--path 要以 / 开头：$PAGE"
API_PATH=${API_PATH:-/${MODULE}s}
[[ $API_PATH =~ ^/ ]] || die "--api-path 要以 / 开头：$API_PATH"

# 驼峰（message_log → MessageLog）；首字段=名称字段
CAMEL=""
IFS='_' read -ra cam_parts <<<"$MODULE"
for p in "${cam_parts[@]}"; do CAMEL+="${p^}"; done
ROUTE=${PAGE#/}
# 前端那个 api 对象名（模块名 snake → 小驼峰 + Api，与仓库里的 dictApi / noticeApi 同风格）
API_OBJ="${CAMEL,}Api"

# 已知字段的现有 i18n 标签（`field.name` → 名称）。命中就能直接用通用键，
# 前端标签、后端 400 文案、apidoc 描述三处都靠它；没命中的字段用字段名当标签。
known_zh() {
    case "$1" in
        name) echo 名称 ;; status) echo 状态 ;; sort) echo 排序 ;; remark) echo 备注 ;;
        code) echo 编码 ;; title) echo 标题 ;; content) echo 内容 ;; nickname) echo 昵称 ;;
        email) echo 邮箱 ;; phone) echo 手机号 ;; username) echo 用户名 ;; type) echo 类型 ;;
        url) echo 链接 ;; icon) echo 图标 ;; value) echo 值 ;; label) echo 标签 ;;
        *) echo "" ;;
    esac
}
rust_type() {
    case "$1" in
        string | text) echo String ;;
        int) echo i32 ;;
        i64) echo i64 ;;
        bool) echo i8 ;;
        datetime) echo NaiveDateTime ;;
        *) echo "" ;;
    esac
}
ts_type() {
    case "$1" in
        string | text | datetime) echo string ;;
        *) echo number ;;
    esac
}

# 字段清单 → 各模板要用的变量（MODEL_FIELDS / CHECKS / COLUMNS / ...）。
# 展开逻辑在 scripts/scaffold-fields.sh（拆开只为守「文件 <500 行」家规）。
build_fields

# ── 落点 ────────────────────────────────────────────────────
# 接口模块落 v1：目录 + mod.rs（与 api/v1/mod.rs 头部约定一致，后续拆分不用搬家）
API_DIR="admin/src/api/v1/$MODULE"
API_RS="$API_DIR/mod.rs"
MODEL_RS="admin/src/models/$MODULE.rs"
TEST_RS="admin/tests/api_${MODULE}_test.rs"
WEB_TS="admin/web/src/api/$MODULE.ts"
WEB_TSX="admin/web/src/pages/system/$MODULE/index.tsx"
API_MOD="admin/src/api/v1/mod.rs"
MODELS_MOD="admin/src/models/mod.rs"

if [ -n "$OUT" ]; then
    ROOT=$PWD
    PREFIX="$OUT/"
    echo "scaffold: --out 模式，写到 $OUT/ 下（跳过 mod.rs 与自检）" >&2
else
    ROOT=$(cd -- "$(dirname -- "$0")/.." 2>/dev/null && pwd -P) || die "找不到仓库根（脚本在 scripts/ 下）"
    PREFIX="$ROOT/"
    # 一个都不许已存在：宁可整个退出，也不能半途留一半生成物
    for f in "$API_RS" "$MODEL_RS" "$TEST_RS" "$WEB_TS" "$WEB_TSX"; do
        [ -e "$ROOT/$f" ] && die "已存在，拒绝覆盖：$f（换个模块名，或先自己删掉它）"
    done
    [ -e "$ROOT/$API_DIR" ] && die "已存在，拒绝覆盖：$API_DIR/（换个模块名，或先自己删掉它）"
    grep -q "^pub mod ${MODULE};" "$ROOT/$API_MOD" && die "$API_MOD 里已有 pub mod $MODULE;"
    grep -q "^pub mod ${MODULE};" "$ROOT/$MODELS_MOD" && die "$MODELS_MOD 里已有 pub mod $MODULE;"
fi

# 页面里真正用到的 antd 组件（按字段类型决定，别引入没用到的组件）
ANTD_IMPORTS="App, Button"
[ "$HAS_DT" = 1 ] && ANTD_IMPORTS+=", DatePicker"
ANTD_IMPORTS+=", Form, Input"
[ "$HAS_NUM" = 1 ] && ANTD_IMPORTS+=", InputNumber"
ANTD_IMPORTS+=", Modal, Popconfirm"
[ "$HAS_BOOL" = 1 ] && ANTD_IMPORTS+=", Select"
ANTD_IMPORTS+=", Space, Table"
YESNO=""
if [ "$HAS_BOOL" = 1 ]; then
    YESNO='  const yesNo = [
    { value: 1, label: t('"'"'common.enabled'"'"') },
    { value: 0, label: t('"'"'common.disabled'"'"') },
  ];

'
fi

# datetime 字段在页面上要折一道：DatePicker 的值类型是 Dayjs，接口契约是字符串。
# 表单的值类型因此在 Form 类型上把那几个字段换成 Dayjs（FormValues），回填 / 提交各转一次。
DAYJS_IMPORT=""
DT_FORM_DECL=""
DT_FORM_TYPE="${CAMEL}Form"
DT_SET_VALUE="form.setFieldsValue(row);"
DT_SUBMIT_BODY="    if (editing) await $API_OBJ.update(editing.id, v);
    else await $API_OBJ.create(v);"
DT_IMPORT=""
TEST_DT_BAD=""
CREATE_DESC_DT=""
if [ "$HAS_DT" = 1 ]; then
    DT_IMPORT="use chrono::NaiveDateTime;
"
    CREATE_DESC_DT="；时间字段格式错回 400"
    DAYJS_IMPORT="import dayjs from 'dayjs';
import type { Dayjs } from 'dayjs';
"
    dt_union=""
    dt_decl=""
    dt_set=""
    dt_to_body=""
    dt_bad=""
    for f in "${DT_FIELDS[@]}"; do
        dt_union+="'$f' | "
        dt_decl+="  $f: Dayjs;
"
        dt_set+="      $f: dayjs(row.$f),
"
        dt_to_body+="      $f: v.$f.format('YYYY-MM-DD HH:mm:ss'),
"
        # 集成测试里的负例：只在「格式不对」上断言 —— 校验发生在进入库之前
        dt_bad+="    // datetime 字段（$f）：格式不对要 400（契约是空格分隔的 \"YYYY-MM-DD HH:MM:SS\"）
    let mut bad_dt = body.clone();
    bad_dt[\"$f\"] = json!(\"2026-01-02T03:04:05\");
    let (st, v) = call(&c, Method::POST, api(\"\"), Some(&token), Some(bad_dt)).await;
    assert_eq!(st, 400, \"$f 格式错要 400：{v}\");

"
    done
    TEST_DT_BAD=$dt_bad
    dt_union=${dt_union% | }
    DT_FORM_DECL="
/** datetime 字段在表单里是 Dayjs（antd DatePicker 的值类型），提交前转回后端契约的
 *  \"YYYY-MM-DD HH:mm:ss\" 字符串 —— 接口类型 ${CAMEL}Form 里这几个字段仍是 string。 */
type FormValues = Omit<${CAMEL}Form, $dt_union> & {
$dt_decl};

"
    DT_FORM_TYPE="FormValues"
    DT_SET_VALUE="form.setFieldsValue({ ...row,
$dt_set    });"
    DT_SUBMIT_BODY="    const body: ${CAMEL}Form = {
      ...v,
$dt_to_body    };
    if (editing) await $API_OBJ.update(editing.id, body);
    else await $API_OBJ.create(body);"
fi

# ── 模板 ────────────────────────────────────────────────────
# 注意：下面是**不带引号**的 heredoc，`$` 与反引号会被 bash 展开 —— 生成物里要写
# 字面量的话得转义（TS 的模板串写成 \` 与 \${id}）。
MODEL_SRC=$(tpl_model)

API_SRC=$(tpl_api)

TEST_SRC=$(tpl_test)

WEB_API_SRC=$(tpl_web_api)

WEB_TSX_SRC=$(tpl_web_page)

# ── 写文件 ──────────────────────────────────────────────────
# 本次真写下的落点（绝对路径）。**先记后写**：`cat` 写一半失败（磁盘满 / 权限 / 目标是坏链接）
# 时，那个半截文件也在名单里，回滚照删。
WROTE=()
BAKDIR=""
ROLLED=0

write_one() { # $1 = 仓库相对路径，stdin = 内容
    local target="$PREFIX$1"
    mkdir -p "$(dirname -- "$target")"
    WROTE+=("$target")
    cat >"$target"
    echo "  生成 $1" >&2
}

# 回滚：还原两个 mod.rs、删掉本次写下的文件与新建的接口/页面目录，最后清掉备份目录。
# 按 WROTE 名单删，不硬编码那 5 条路径 —— 中途失败时可能只写了前几个，`--out` 模式的
# 落点还在仓库外。幂等（ROLLED 闸）：显式调用的失败分支与下面的 EXIT 兜底可能各来一次。
rollback() {
    [ "$ROLLED" = 0 ] || return 0
    ROLLED=1
    if [ -n "$BAKDIR" ] && [ -d "$BAKDIR" ]; then
        cp "$BAKDIR/api_mod.rs" "$ROOT/$API_MOD"
        cp "$BAKDIR/models_mod.rs" "$ROOT/$MODELS_MOD"
    fi
    local f
    for f in ${WROTE[@]+"${WROTE[@]}"}; do rm -f -- "$f"; done
    rm -f -- "$ROOT/$API_MOD.tmp" "$ROOT/$MODELS_MOD.tmp"
    [ -n "$OUT" ] || rmdir "$ROOT/$API_DIR" 2>/dev/null || true
    [ -n "$OUT" ] || rmdir "$ROOT/admin/web/src/pages/system/$MODULE" 2>/dev/null || true
    [ -z "$BAKDIR" ] || rm -rf -- "$BAKDIR"
    local what="本次写下的文件已删"
    [ -n "$OUT" ] || what="两个 mod.rs 还原、$what" # --out 模式压根没碰 mod.rs
    echo "scaffold: 已回滚本次生成（$what）" >&2
}

# 兜底：写盘 I/O 失败、自检不过、Ctrl-C —— 任何非零退出都走同一条回滚路。
# 装在参数校验与「已存在」闸之后：那之前退出时还什么都没写，不该动工作树。
cleanup_on_exit() {
    local rc=$?
    [ "$rc" -eq 0 ] || rollback
    exit "$rc"
}
# Ctrl-C / kill 也不能把半套生成物留在工作树里：信号先转成非零退出码，再走同一个
# EXIT 兜底。直接挂 INT/TERM 的话，信号恰好落在一条成功的命令之后时 `$?` 是 0，
# 会被 cleanup_on_exit 当成成功退出而**跳过回滚**（release.sh 同款处理）。
trap cleanup_on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# 用 here-string 而不是管道：管道右侧是子 shell，write_one 里往 WROTE 记的落点会丢在
# 子 shell 里 —— 那正是「回滚时一个文件都没删」的成因（记账必须在当前 shell 生效）。
write_one "$MODEL_RS" <<<"$MODEL_SRC"
write_one "$API_RS" <<<"$API_SRC"
write_one "$TEST_RS" <<<"$TEST_SRC"
write_one "$WEB_TS" <<<"$WEB_API_SRC"
write_one "$WEB_TSX" <<<"$WEB_TSX_SRC"

# mod.rs 追加声明：不加，上面的 .rs 根本不参与编译，自检就是假绿。
# - api/v1/mod.rs 只插 `pub mod`，位置在最后一条 `pub mod` 之后（声明聚成一块）；
#   不碰 `pub use v1::*;`——那是 api/mod.rs 的稳定兼容路径，不是模块声明块。
# - models/mod.rs 插 `pub mod` 与 `pub use`（两个块各自最后一条之后，保持原排序）。
if [ -z "$OUT" ]; then
    # 自检失败要能整体回退：先给两个 mod.rs 留底（放临时目录，别在仓库里留垃圾）
    BAKDIR=$(mktemp -d) || die "建不了临时目录"
    cp "$ROOT/$API_MOD" "$BAKDIR/api_mod.rs"
    cp "$ROOT/$MODELS_MOD" "$BAKDIR/models_mod.rs"
    awk -v line="pub mod $MODULE;" '
        { buf[NR] = $0 }
        /^pub mod / { last_mod = NR }
        END {
            for (i = 1; i <= NR; i++) {
                print buf[i];
                if (i == last_mod) print line;
            }
        }' "$ROOT/$API_MOD" >"$ROOT/$API_MOD.tmp"
    mv "$ROOT/$API_MOD.tmp" "$ROOT/$API_MOD"
    awk -v line="pub mod $MODULE;" -v use_line="pub use $MODULE::$CAMEL;" '
        { buf[NR] = $0 }
        /^pub mod / { last_mod = NR }
        /^pub use / { last_use = NR }
        END {
            for (i = 1; i <= NR; i++) {
                print buf[i];
                if (i == last_mod) print line;
                if (i == last_use) print use_line;
            }
        }' "$ROOT/$MODELS_MOD" >"$ROOT/$MODELS_MOD.tmp"
    mv "$ROOT/$MODELS_MOD.tmp" "$ROOT/$MODELS_MOD"
    echo "  追加 $API_MOD → pub mod $MODULE;" >&2
    echo "  追加 $MODELS_MOD → pub mod $MODULE; / pub use $MODULE::$CAMEL;" >&2
fi

# ── 打印「剩下的手工接线」（清单文本在 scripts/scaffold-wiring.sh）────
print_wiring

# ── 自检：生成物必须自己先编译过 ────────────────────────────
if [ -n "$OUT" ]; then
    cat <<EOF
── --out 模式：跳过编译自检（生成物在仓库外，不参与编译）。
   想验证脚本本身没坏，去掉 --out 用个真模块名跑一遍。
EOF
    exit 0
fi

cd "$ROOT"
# 自检失败 = 脚本的 bug：rollback 把这次写的东西全撤掉（EXIT 兜底也会走同一条路，
# 这里显式调一次只为把话说在 die 前面）。半截生成物比重跑一次贵得多 ——
# 它会占着落点，下次跑被「已存在，拒绝覆盖」挡住，还得手工清。
echo "── 自检 1/2：cargo check -p bee_admin --all-targets（前台跑，几分钟）" >&2
cargo check -p bee_admin --all-targets --manifest-path "$ROOT/Cargo.toml" ||
    { rollback; die "生成物编译不过 = 脚本的 bug。修 scripts/scaffold-module.sh，别手改生成物。"; }
echo "── 自检 2/2：pnpm -C admin/web exec tsc -b" >&2
pnpm -C "$ROOT/admin/web" exec tsc -b ||
    { rollback; die "生成的前端编译不过 = 脚本的 bug。修 scripts/scaffold-module.sh，别手改生成物。"; }
rm -rf "$BAKDIR"
BAKDIR=""
echo "── 自检通过：$MODULE 骨架已就位（上面 ①③ 两条粘完就能调接口）"
