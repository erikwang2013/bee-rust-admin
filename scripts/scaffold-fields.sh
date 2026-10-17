#!/usr/bin/env bash
# 字段清单 → 生成变量的展开（由 scripts/scaffold-module.sh source 进来用，别单独跑）。
# 多行字符串里的缩进会逐字进模板，函数体不要重新缩进。
build_fields() {
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
# 页面按需 import：字段里没有数字/布尔/datetime 时引 InputNumber/Select/DatePicker 会是没用到的导入
HAS_NUM=0
HAS_BOOL=0
HAS_DT=0

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
        string | text | int | i64 | bool | datetime) ;;
        *) die "字段类型只认 string/text/int/i64/bool/datetime：$spec" ;;
    esac
    flen=${flen:-128}
    [[ $flen =~ ^[0-9]+$ ]] || die "字段长度要是数字：$spec"
    [ "$flen" -ge 1 ] && [ "$flen" -le 20000 ] || die "字段长度要在 1..20000：$spec"
    zh=$(known_zh "$fname")
    label=$([ -n "$zh" ] && echo "t('field.$fname')" || echo "'$fname'")
    # 中文说法：命中现有词表就用它，没命中退回字段名（400 文案 / apidoc 描述都用它）
    FIELD_ZH=${zh:-$fname}
    # 第一个 string 字段当选名称字段：列表模糊查、必填校验、超长校验都挂它
    if [ -z "$NAME_FIELD" ] && [ "$ftype" = string ]; then
        NAME_FIELD=$fname
        NAME_MAX=$flen
    fi

    # 每个类型只在这里声明差异，下面的追加统一做（曾经按类型抄了四遍，改一处要改四处）
    API_TY=""         # 接口面的类型（请求体 / 查询参数）——只有 datetime 与模型面不同
    ADOC_SUFFIX=""    # apidoc 描述尾巴（datetime 要带上格式）
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
        datetime)
            HAS_DT=1
            DT_FIELDS+=("$fname")
            # 模型面是 NaiveDateTime，接口面是字符串：chrono 的原生 serde 只认 "T" 分隔，
            # 本仓契约是空格分隔的 "YYYY-MM-DD HH:MM:SS"（见 docs/scaffold.md）
            API_TY=String
            RUST_TY=NaiveDateTime
            TS_TY=string
            TS_QUERY_TY=string
            ADOC_TY=string
            ADOC_SUFFIX="（YYYY-MM-DD HH:MM:SS）"
            FILTER=dt
            DEFAULT_V='"2026-01-02 03:04:05"'
            FORM_ITEM='<DatePicker showTime style={{ width: '\''100%'\'' }} />'
            MODEL_DECL="    /// $FIELD_ZH（DATETIME，秒精度；接口收发都是 \"YYYY-MM-DD HH:MM:SS\" 字符串）
    #[serde(serialize_with = \"crate::util::ser_dt\")]
    pub $fname: NaiveDateTime,"
            CHECKS+="
/// $FIELD_ZH的解析：契约是 \"YYYY-MM-DD HH:MM:SS\"（空格分隔、秒精度 —— 与本仓其它时间字段
/// 一个口径）。收字符串再显式解析，是因为 chrono 的 Deserialize 只认 \`T\` 分隔：直接拿
/// NaiveDateTime 当请求体字段，前端按契约传空格分隔反而被判 400。
/// 解析失败回 400（动态文案、无稳定错误码，前端回落 msg）。
fn parse_$fname(value: &str) -> Result<NaiveDateTime, ApiError> {
    NaiveDateTime::parse_from_str(value.trim(), \"%Y-%m-%d %H:%M:%S\")
        .map_err(|_| ApiError::BadRequest(format!(\"{}格式需为 YYYY-MM-DD HH:MM:SS\", \"$FIELD_ZH\")))
}
"
            # 校验放在发号之前（与名称字段的 check 同一位置）：格式错不消耗 id
            CHECK_CALLS+="    parse_$fname(&body.$fname)?;
"
            CHECK_TESTS+="
    #[test]
    fn parse_${fname}_accepts_contract_format_only() {
        assert!(parse_$fname(\"2026-01-02 03:04:05\").is_ok());
        assert!(parse_$fname(\"  2026-01-02 03:04:05  \").is_ok(), \"两侧空白 trim 后放行\");
        for bad in [\"2026-01-02T03:04:05\", \"2026-01-02\", \"2026-01-02 03:04\", \"\", \"2026-02-30 00:00:00\"] {
            assert!(parse_$fname(bad).is_err(), \"格式不对要拒：{bad}\");
        }
    }
"
            ;;
    esac
    # 没单列声明接口面类型的（除 datetime 外都如此）：两边同型
    API_TY=${API_TY:-$RUST_TY}

    MODEL_FIELDS+="$MODEL_DECL
"
    BODY_FIELDS+="    pub $fname: $API_TY,
"
    QUERY_FIELDS+="    pub $fname: Option<$API_TY>,
"
    TS_ROW+="  $fname: $TS_TY;
"
    TS_FORM+="  $fname: $TS_TY;
"
    TS_QUERY+=" $fname?: $TS_QUERY_TY;"
    ADOC_DESC=$FIELD_ZH
    ADOC_PARAMS+="#[apidoc::param(name = \"$fname\", ty = \"$ADOC_TY\", required, desc = \"$ADOC_DESC$ADOC_SUFFIX\")]
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
    elif [ "$FILTER" = dt ]; then
        # 查询参数同样是字符串：解析成 NaiveDateTime 再当绑定参数（筛选值是用户输入，不进 SQL 串）
        QUERY_FILTERS+="    if let Some(v) = q.$fname.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_eq(\"$fname\", parse_$fname(v)?).map_err(ApiError::from)?;
    }
"
        ADOC_QUERIES+="#[apidoc::query(name = \"$fname\", ty = \"string\", desc = \"$ADOC_DESC（精确匹配，格式 YYYY-MM-DD HH:MM:SS）\")]
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
        # 接口面与模型面不同型（datetime）：赋值处顺手转一次，create/update 都不会把原串塞进库
        ASSIGN_EXPR="body.$fname"
        [ "$API_TY" = "$RUST_TY" ] || ASSIGN_EXPR="parse_$fname(&body.$fname)?"
        CREATE_ASSIGN+="        $fname: $ASSIGN_EXPR,
"
        UPDATE_ASSIGN+="    row.$fname = $ASSIGN_EXPR;
"
        rules=""
        [ "$RUST_TY" != String ] && rules="
            rules={[{ required: true, message: t('validate.required', { field: $label }) }]}"
        FORM_ITEMS+="          <Form.Item name=\"$fname\" label={$label}$rules>
            $FORM_ITEM
          </Form.Item>
"
    fi
    case "$ftype" in
        bool)
            COLUMNS+="    {
      title: $label, dataIndex: '$fname', width: 90,
      render: (v: number) => (v === 1 ? t('common.enabled') : t('common.disabled')),
    },
" ;;
        # 时间串是 YYYY-MM-DD HH:MM:SS（19 字符），给足宽度别折行（与 created_at 列同宽）
        datetime) COLUMNS+="    { title: $label, dataIndex: '$fname', width: 170 },
" ;;
        *) COLUMNS+="    { title: $label, dataIndex: '$fname' },
" ;;
    esac
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
}
