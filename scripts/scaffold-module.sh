#!/usr/bin/env bash
# 模块脚手架（扩展工程项 10）：一条命令从一个模块名 + 权限前缀 + 字段清单生成模块骨架。
#
#   scripts/scaffold-module.sh <模块名> <权限前缀> [字段...] [选项]
#   scripts/scaffold-module.sh message system:message name:string:64 status:bool sort:int remark:text
#
# 字段写法 `<名>:<类型>[:<长度>]`，类型：string（VARCHAR，默认 128）/ text（TEXT）/
# int（i32）/ i64 / bool（i8，1 是 / 0 否）。**第一个 string 字段被当成名称字段**：
# 列表的模糊查询、新建的必填与超长校验都只挂在它身上。
#
# 生成物（骨架，不是成品）：模型 / 接口（4 个 CRUD 接口 + apidoc 注解）/ 单元测试 /
# 集成测试 / 前端 api 封装 / 前端页面。剩下的「真实业务逻辑」自己往里头长。
#
# 三条硬规则：
# 1. 生成物**必须自己先编译过**：跑完原地执行 `cargo check -p bee_admin --all-targets` +
#    `pnpm -C admin/web exec tsc -b`；编译不过 = 脚本的 bug，修脚本别改生成物糊过去
#    （`--out` 跳过自检：那只是把骨架写到仓库外对照用，不参与编译）。自检不过就**整体回滚**
#    （两个 mod.rs 还原、本次生成物删掉），工作树回到跑脚本之前。
# 2. **不覆盖任何已有文件**：任一目标已存在就整体退出（一次误跑会清掉线上模块）。
# 3. 只追加 `api/mod.rs` 与 `models/mod.rs` 各一行 `pub mod`（不加，生成物不参与编译，
#    自检就是假绿）；路由/菜单种子/前端路由/i18n 只**打印可粘贴片段** —— 结构化编辑
#    脚本改错比不改更糟，位置只有人知道。
#
# 想先看骨架长什么样：`--out /tmp/sk` 重造既有模块做对照（如用字典管理的字段清单重造 dict）。
set -euo pipefail

die() { echo "scaffold: $*" >&2; exit 1; }

# 模板在 scripts/scaffold-templates.sh（同 shell source 进来，heredoc 的展开语义一样；
# 拆开只为守住「文件 <500 行」的家规）。$ 展开与反引号转义在那边照旧。
SCRIPT_DIR=$(cd -- "$(dirname -- "$0")" 2>/dev/null && pwd -P) || die "定位不到脚本所在目录"
[ -f "$SCRIPT_DIR/scaffold-templates.sh" ] || die "缺脚本模板：$SCRIPT_DIR/scaffold-templates.sh"
. "$SCRIPT_DIR/scaffold-templates.sh"

usage() {
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
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
        *) echo "" ;;
    esac
}
ts_type() {
    case "$1" in
        string | text) echo string ;;
        *) echo number ;;
    esac
}


MODEL_FIELDS=""
BODY_FIELDS=""
QUERY_FIELDS=""
QUERY_FILTERS=""
CREATE_ASSIGN=""
UPDATE_ASSIGN=""
TS_ROW=""
TS_FORM=""
TS_QUERY=""
FORM_ITEMS=""
COLUMNS=""
CHECKS=""
CHECK_CALLS=""
CHECK_TESTS=""
ADOC_PARAMS=""
ADOC_QUERIES=""
I18N_ZH=""
I18N_EN=""
DEFAULTS=""
NAME_FIELD=""
NAME_MAX=128
# 见过的字段名，带竖线包裹（"|a|b|"）：查重比数组省事，也不吃 set -u 的亏
SEEN_FIELDS="|"
# 页面按需 import：字段里没有数字/布尔时引 InputNumber/Select 会是没用到的导入
HAS_NUM=0
HAS_BOOL=0

for spec in "${FIELDS[@]}"; do
    IFS=':' read -r fname ftype flen <<<"$spec"
    ftype=${ftype:-string}
    [[ $fname =~ ^[a-z][a-z0-9_]*$ ]] || die "字段名只能是小写字母/数字/下划线且字母开头：$spec"
    # 保留名：id/created_at/updated_at 模型固定生成，page/size 是分页参数（Rust Query 结构体与
    # TS 查询接口里都已有同名成员）—— 让它们进来就是重复列 / 重复字段，生成物必然编译不过
    case "$fname" in
        id | created_at | updated_at | page | size)
            die "字段名是保留名：$fname（id/created_at/updated_at 由模型固定生成，page/size 是分页参数）" ;;
    esac
    case "$SEEN_FIELDS" in
        *"|$fname|"*) die "字段名重复：$fname（同一个字段写两遍 = 重复列）" ;;
    esac
    SEEN_FIELDS="$SEEN_FIELDS$fname|"
    case "$ftype" in
        string | text | int | i64 | bool) ;;
        *) die "字段类型只认 string/text/int/i64/bool：$spec" ;;
    esac
    flen=${flen:-128}
    [[ $flen =~ ^[0-9]+$ ]] || die "字段长度要是数字：$spec"
    [ "$flen" -ge 1 ] && [ "$flen" -le 20000 ] || die "字段长度要在 1..20000：$spec"
    zh=$(known_zh "$fname")
    label=$([ -n "$zh" ] && echo "t('field.$fname')" || echo "'$fname'")
    # 第一个 string 字段当选名称字段：列表模糊查、必填校验、超长校验都挂它
    if [ -z "$NAME_FIELD" ] && [ "$ftype" = string ]; then
        NAME_FIELD=$fname
        NAME_MAX=$flen
    fi

    # 每个类型只在这里声明差异，下面的追加统一做（曾经按类型抄了四遍，改一处要改四处）
    case "$ftype" in
        string | text)
            RUST_TY=String
            TS_TY=string
            ADOC_TY=string
            FILTER=contains
            TS_QUERY_TY=string
            if [ "$ftype" = string ]; then
                MODEL_DECL="    #[bee(sql_type = \"VARCHAR($flen)\")]
    pub $fname: String,"
                # `|| true` 不能省：set -e 下赋值里的 `$(失败的 test && …)` 会让整个赋值返回 1，
                # 脚本静默退出（rc=1、零输出零落盘）—— 第二个及以后的 string 字段必然踩到
                FORM_ITEM="<Input$([ "$fname" = "$NAME_FIELD" ] && echo " maxLength={$flen}" || true) />"
            else
                MODEL_DECL="    #[bee(sql_type = \"TEXT\")]
    pub $fname: String,"
                FORM_ITEM="<Input.TextArea rows={4} />"
            fi
            DEFAULT_V="format!(\"{unique}-$fname\")"
            # 文本列一律在入口卡长度：不卡就是 MySQL 1406 → 500（与公告的 check_title/check_content 同款）
            if [ "$ftype" = string ]; then
                len_max=$flen
                len_doc="/// $([ -n "$zh" ] && echo "$zh" || echo "$fname")的列长（与模型里的 VARCHAR 对齐）"
            else
                len_max=20000
                len_doc="/// TEXT 列是 64KB 字节，按中文 3 字节算 20000 字仍在列内（与公告正文一个口径）"
            fi
            if [ "$fname" = "$NAME_FIELD" ]; then
                NAME_MAX=$len_max
            else
                CHECK_CALLS+="    check_${fname}(&body.$fname)?;
"
                CHECK_TESTS+="
    #[test]
    fn check_${fname}_rejects_too_long() {
        assert!(check_${fname}(\"\").is_ok(), \"空值允许（空串是合法的内容）\");
        assert!(check_${fname}(&\"x\".repeat(${fname^^}_MAX)).is_ok());
        assert!(check_${fname}(&\"x\".repeat(${fname^^}_MAX + 1)).is_err());
    }
"
            fi
            CHECKS+="
$len_doc
const ${fname^^}_MAX: usize = $len_max;
"
            if [ "$fname" != "$NAME_FIELD" ]; then
                CHECKS+="
fn check_${fname}(value: &str) -> Result<(), ApiError> {
    check_len(\"$fname\", value, ${fname^^}_MAX)
}
"
            fi
            ;;
        int | i64)
            HAS_NUM=1
            RUST_TY=$([ "$ftype" = int ] && echo i32 || echo i64)
            MODEL_DECL="    pub $fname: $RUST_TY,"
            TS_TY=number
            TS_QUERY_TY=number
            ADOC_TY=int
            FILTER=eq
            DEFAULT_V=1
            FORM_ITEM='<InputNumber style={{ width: '\''100%'\'' }} />'
            ;;
        bool)
            HAS_BOOL=1
            RUST_TY=i8
            MODEL_DECL="    /// 1 是 / 0 否
    pub $fname: i8,"
            TS_TY=number
            TS_QUERY_TY=number
            ADOC_TY=int
            FILTER=eq
            DEFAULT_V=1
            FORM_ITEM='<Select options={yesNo} />'
            ;;
    esac

    MODEL_FIELDS+="$MODEL_DECL
"
    BODY_FIELDS+="    pub $fname: $RUST_TY,
"
    QUERY_FIELDS+="    pub $fname: Option<$RUST_TY>,
"
    TS_ROW+="  $fname: $TS_TY;
"
    TS_FORM+="  $fname: $TS_TY;
"
    TS_QUERY+=" $fname?: $TS_QUERY_TY;"
    ADOC_DESC="$([ -n "$zh" ] && echo "$zh" || echo "$fname")"
    ADOC_PARAMS+="#[apidoc::param(name = \"$fname\", ty = \"$ADOC_TY\", required, desc = \"$ADOC_DESC\")]
"
    # 名称字段的测试值直接用唯一串：列表要按它查得到
    if [ "$fname" = "$NAME_FIELD" ]; then
        DEFAULTS+=" \"$fname\": unique,"
    else
        DEFAULTS+=" \"$fname\": $DEFAULT_V,"
    fi
    if [ "$FILTER" = contains ]; then
        QUERY_FILTERS+="    if let Some(v) = q.$fname.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains(\"$fname\", v.trim()).map_err(ApiError::from)?;
    }
"
        ADOC_QUERIES+="#[apidoc::query(name = \"$fname\", ty = \"string\", desc = \"$ADOC_DESC（模糊匹配）\")]
"
    else
        QUERY_FILTERS+="    if let Some(v) = q.$fname {
        qs = qs.filter_eq(\"$fname\", v).map_err(ApiError::from)?;
    }
"
        ADOC_QUERIES+="#[apidoc::query(name = \"$fname\", ty = \"int\", desc = \"$ADOC_DESC\")]
"
    fi
    # 名称字段（必填 + trim）、bool（1/0 显成启用/禁用）才要特殊处理
    if [ "$fname" = "$NAME_FIELD" ]; then
        CREATE_ASSIGN+="        $fname: body.$fname.trim().to_string(),
"
        UPDATE_ASSIGN+="    row.$fname = body.$fname.trim().to_string();
"
        FORM_ITEMS+="          <Form.Item
            name=\"$fname\"
            label={$label}
            rules={[{ required: true, whitespace: true, message: t('validate.required', { field: $label }) }]}
          >
            $FORM_ITEM
          </Form.Item>
"
    else
        CREATE_ASSIGN+="        $fname: body.$fname,
"
        UPDATE_ASSIGN+="    row.$fname = body.$fname;
"
        rules=""
        [ "$RUST_TY" != String ] && rules="
            rules={[{ required: true, message: t('validate.required', { field: $label }) }]}"
        FORM_ITEMS+="          <Form.Item name=\"$fname\" label={$label}$rules>
            $FORM_ITEM
          </Form.Item>
"
    fi
    if [ "$ftype" = bool ]; then
        COLUMNS+="    {
      title: $label, dataIndex: '$fname', width: 90,
      render: (v: number) => (v === 1 ? t('common.enabled') : t('common.disabled')),
    },
"
    else
        COLUMNS+="    { title: $label, dataIndex: '$fname' },
"
    fi
    # 字段名不在现有词表里时，附一份 i18n 占位（粘不粘都行，见 docs/scaffold.md）
    if [ -z "$zh" ]; then
        I18N_ZH+="  '$MODULE.$fname': '$fname',
"
        I18N_EN+="  '$MODULE.$fname': '$fname',
"
    fi
done

[ -n "$NAME_FIELD" ] || die "字段清单里至少要有一个 string 字段（当名称字段用），例如 name:string:64"
# 名称字段的中文说法（命中现有词表就用它，没命中就退回字段名）
NAME_ZH=$(known_zh "$NAME_FIELD")
NAME_ZH=${NAME_ZH:-$NAME_FIELD}

# ── 落点 ────────────────────────────────────────────────────
API_RS="admin/src/api/$MODULE.rs"
MODEL_RS="admin/src/models/$MODULE.rs"
TEST_RS="admin/tests/api_${MODULE}_test.rs"
WEB_TS="admin/web/src/api/$MODULE.ts"
WEB_TSX="admin/web/src/pages/system/$MODULE/index.tsx"
API_MOD="admin/src/api/mod.rs"
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
    grep -q "^pub mod ${MODULE};" "$ROOT/$API_MOD" && die "$API_MOD 里已有 pub mod $MODULE;"
    grep -q "^pub mod ${MODULE};" "$ROOT/$MODELS_MOD" && die "$MODELS_MOD 里已有 pub mod $MODULE;"
fi

# 页面里真正用到的 antd 组件（按字段类型决定，别引入没用到的组件）
ANTD_IMPORTS="App, Button, Form, Input"
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

# ── 模板 ────────────────────────────────────────────────────
# 注意：下面是**不带引号**的 heredoc，`$` 与反引号会被 bash 展开 —— 生成物里要写
# 字面量的话得转义（TS 的模板串写成 \` 与 \${id}）。
MODEL_SRC=$(tpl_model)

API_SRC=$(tpl_api)

TEST_SRC=$(tpl_test)

WEB_API_SRC=$(tpl_web_api)

WEB_TSX_SRC=$(tpl_web_page)

# ── 写文件 ──────────────────────────────────────────────────
write_one() { # $1 = 仓库相对路径，stdin = 内容
    local target="$PREFIX$1"
    mkdir -p "$(dirname -- "$target")"
    cat >"$target"
    echo "  生成 $1" >&2
}

printf '%s\n' "$MODEL_SRC" | write_one "$MODEL_RS"
printf '%s\n' "$API_SRC" | write_one "$API_RS"
printf '%s\n' "$TEST_SRC" | write_one "$TEST_RS"
printf '%s\n' "$WEB_API_SRC" | write_one "$WEB_TS"
printf '%s\n' "$WEB_TSX_SRC" | write_one "$WEB_TSX"

# mod.rs 各追加一行：不加，上面两个 .rs 根本不参与编译，自检就是假绿。
# 插在最后一条同类语句之后（保持「声明聚成一块」的形状，不打断现有排序）。
if [ -z "$OUT" ]; then
    # 自检失败要能整体回退：先给两个 mod.rs 留底（放临时目录，别在仓库里留垃圾）
    BAKDIR=$(mktemp -d) || die "建不了临时目录"
    cp "$ROOT/$API_MOD" "$BAKDIR/api_mod.rs"
    cp "$ROOT/$MODELS_MOD" "$BAKDIR/models_mod.rs"
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

# 自检失败 = 脚本的 bug：把这次写的东西全撤掉，工作树回到跑脚本之前。
# 半截生成物比重跑一次贵得多 —— 它会占着落点，下次跑被「已存在，拒绝覆盖」挡住，还得手工清。
rollback() {
    cp "$BAKDIR/api_mod.rs" "$ROOT/$API_MOD"
    cp "$BAKDIR/models_mod.rs" "$ROOT/$MODELS_MOD"
    rm -f "$ROOT/$API_RS" "$ROOT/$MODEL_RS" "$ROOT/$TEST_RS" "$ROOT/$WEB_TS" "$ROOT/$WEB_TSX"
    rmdir "$ROOT/admin/web/src/pages/system/$MODULE" 2>/dev/null || true
    rm -rf "$BAKDIR"
    echo "scaffold: 已回滚本次生成（两个 mod.rs 还原、5 个生成物已删）" >&2
}

# ── 打印「剩下的手工接线」──────────────────────────────────
cat <<EOF

── 接线清单（脚本不替你做，逐条粘）───────────────────────────

① 建表：admin/src/seed.rs 的 migrate 清单里加一行（不会删列，安全）
    migrate::sync::<crate::models::$CAMEL, _>(db).await?;

② 菜单种子：同一个文件的 BUILTIN_MENUS 里加一条（权限码就是从这里发出来的）
    (
        "$LABEL",
        "$PAGE",
        "$ROUTE/index",
        "ProfileOutlined",
        "$PERM",
        &["list", "add", "edit", "remove"],
    ),

③ 路由挂载：admin/src/main.rs 的路由链上，任选合适位置（别塞进别的 ns 里）
    .ns("/api/v1$API_PATH", |ns| {
        ns.get("", api::$MODULE::list)
            .post("", api::$MODULE::create)
            .put("/{id}", api::$MODULE::update)
            .delete("/{id}", api::$MODULE::remove)
    })
    （挂完把 src/api/$MODULE.rs 与 src/models/$MODULE.rs 顶上那两行 #![allow(dead_code)] 删掉：
     它们是「还没接线」这段过渡期压编译警告用的，接完线就该没了）

④ 前端路由：admin/web/src/router.tsx
    const ${CAMEL}Page = lazy(() => import('./pages/system/$MODULE/index'));
    // 路由表里（<L> 是登录守卫）：
    { path: '$ROUTE', element: <RequirePerm code="$PERM:list"><L><${CAMEL}Page /></L></RequirePerm> },

⑤ 前端图标：页面菜单要显示的话，admin/src/seed.rs 里用的图标名必须在
    admin/web/src/icons.ts 的白名单里（ProfileOutlined 已有）。

⑥ 集成测试：admin/tests/common/mod.rs 的 reset_db 表清单里加 "$MODULE"
    （不加的话上一轮跑的数据会留到下一轮；生成的测试用唯一名字，不会因此假失败）。

EOF

if [ -n "$I18N_ZH" ] || [ -n "$I18N_EN" ]; then
    cat <<EOF
⑦ i18n（可选：下面这些字段没命中现有的 field.* 键，页面先用字段名当标签；
    想要中文/英文标签就粘进两张词表 —— 键必须两边一一对应，缺一边 tsc 就报错）
    admin/web/src/i18n/zh-CN.ts：
$I18N_ZH  '$MODULE.hint': '$LABEL —— 使用说明占位，换成真的。',
    admin/web/src/i18n/en-US.ts：
$I18N_EN  '$MODULE.hint': '$LABEL — replace this hint.',
    粘完把页面里的字面量标签换成 t('$MODULE.<字段>')。

EOF
else
    cat <<EOF
⑦ i18n：本模块的字段全部命中现有 field.* 键，无需改词表
    （想要模块专属文案再自己加 $MODULE.* 键，两张词表要同时加）。

EOF
fi

# ── 自检：生成物必须自己先编译过 ────────────────────────────
if [ -n "$OUT" ]; then
    cat <<EOF
── --out 模式：跳过编译自检（生成物在仓库外，不参与编译）。
   想验证脚本本身没坏，去掉 --out 用个真模块名跑一遍。
EOF
    exit 0
fi

cd "$ROOT"
echo "── 自检 1/2：cargo check -p bee_admin --all-targets（前台跑，几分钟）" >&2
cargo check -p bee_admin --all-targets --manifest-path "$ROOT/Cargo.toml" ||
    { rollback; die "生成物编译不过 = 脚本的 bug。修 scripts/scaffold-module.sh，别手改生成物。"; }
echo "── 自检 2/2：pnpm -C admin/web exec tsc -b" >&2
pnpm -C "$ROOT/admin/web" exec tsc -b ||
    { rollback; die "生成的前端编译不过 = 脚本的 bug。修 scripts/scaffold-module.sh，别手改生成物。"; }
rm -rf "$BAKDIR"
echo "── 自检通过：$MODULE 骨架已就位（上面 ①③ 两条粘完就能调接口）"
