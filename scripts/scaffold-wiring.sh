# 接线清单：脚本跑完打印给用户逐条粘的那几段（生成器不替人做结构化编辑，见
# scaffold-module.sh 头部规则 3）。与 scaffold-templates.sh 同样 source 进主脚本 ——
# heredoc 的展开语义与写在同一个文件里完全一致，拆开只为守「文件 <500 行」家规。
#
# 函数体里的 heredoc 特意顶格（<<EOF 与终止符都不缩进）：缩进会原样进到打印内容里，
# 而这份清单是给人直接粘进源码的，多一个空格都不该出现。
print_wiring() {
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
}
