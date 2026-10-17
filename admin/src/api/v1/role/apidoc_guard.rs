// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

/// 钉住 `data_scope` 的取值说明：它描述的是**别的模块的行为**
/// （`crate::datascope::resolve` + 前端 `SCOPE_KEYS`），而文档站对外可见 ——
/// 曾经写反过（2/4/5 三档张冠李戴），没人会去核对，于是一直错着。
/// 这里从 apidoc 的注册表里把真实产出的文案捞回来断言，改错就会红。
#[test]
fn api_doc_data_scope_matches_the_mapping() {
    // 递归走 children：角色详情/新增的响应体里 `data_scope` 是**嵌套**的字段，
    // 只扫顶层会漏掉一半（我第一次写就只捞到 2 个，被下面的计数断言拦住了）
    fn walk<'a>(ps: &'a [apidoc::DocParam], out: &mut Vec<&'a apidoc::DocParam>) {
        for p in ps {
            out.push(p);
            walk(p.children, out);
        }
    }
    let mut seen = 0;
    for ep in apidoc::DocRegistry::collect() {
        let mut all = Vec::new();
        walk(&ep.params, &mut all);
        walk(&ep.querys, &mut all);
        walk(&ep.returned, &mut all);
        for p in all {
            if p.name != "data_scope" {
                continue;
            }
            seen += 1;
            let desc = p.desc.unwrap_or_default();
            assert_eq!(
                desc, "数据范围：1 全部 / 2 本部门及以下 / 3 本部门 / 4 仅本人 / 5 自定义",
                "apidoc 里的 data_scope 说明与 datascope::resolve 的实际分支不符（端点 {}）",
                ep.url
            );
        }
    }
    assert!(seen >= 4, "只在 {seen} 个端点里找到 data_scope，注解是不是被删了？");
}
