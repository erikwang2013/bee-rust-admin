// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 附件中心（项 5）：分片上传 / 列表 / 下载（登录 + 数据权限）/ 删除 + 公告配图的多态关联。
//!
//! 起真实进程 + 真库（bee_admin_test），需要 BEE_ADMIN_DB_DSN；上传根是
//! `conf/app.conf.test` 里的 `target/test-uploads`（服务进程的 cwd 与本测试相同）。
//!
//! 「公开」的口径（本文件是它的权威断言）：附件绑定了**已发布**的公告 = 公开
//! （公告对全体登录用户可见，配图必须跟着可见）；其余一律按上传人的数据权限卡。
mod common;

use reqwest::Method;
use serde_json::json;

#[path = "api_attach_test/helpers.rs"]
mod helpers;
use helpers::{
    attach_file, call, chunk_form, find_perm, jpeg_bytes, orphan_cleanup_only_collects_unreferenced_files,
    png_bytes, post_form, preprocess_form, tree_files, upload_attach, PNG_SHA256,
};

#[tokio::test]
async fn v19_attachment_center() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    let _ = std::fs::remove_dir_all("target/test-uploads/attach");

    let (_server, base) = common::start_server().await;
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1{p}");
    let pool = sqlx::MySqlPool::connect(&dsn).await.unwrap();
    let admin = common::login(&base, "admin", "admin123").await.expect("超管登录");
    let admin_id = common::my_id(&base, &admin).await;
    let admin_num = common::dec_id(&admin_id);

    // ── 1. 鉴权：上传口、列表、下载都要求登录 ───────────────────────
    let (st, v) = call(&c, Method::GET, api("/attachments"), None, None).await;
    assert_eq!(st, 401, "列表要登录: {v}");
    let png = png_bytes();
    let (st, v) = post_form(
        &c,
        api("/attachments/upload/preprocess"),
        None,
        preprocess_form("logo.png", png.len(), &aetherupload::md5_hex(&png), &[]),
    )
    .await;
    assert_eq!(st, 401, "preprocess 要登录: {v}");
    assert_eq!(v["err"], "auth.unauthorized", "未登录信封: {v}");

    // ── 2. 上传：两片，末片才落行 ──────────────────────────────────
    let hash = aetherupload::md5_hex(&png);
    let (st, v) = post_form(
        &c,
        api("/attachments/upload/preprocess"),
        Some(&admin),
        preprocess_form("logo.png", png.len(), &hash, &[]),
    )
    .await;
    assert_eq!(st, 200, "预处理: {v}");
    let d = &v["data"];
    assert_eq!(d["resource_ext"], "png", "扩展名来自原文件名: {v}");
    assert_eq!(d["saved_path"], "", "新进程的秒传索引是空的: {v}");
    let chunk_size = d["chunk_size"].as_u64().unwrap() as usize;
    assert!(chunk_size > 0 && chunk_size < 10 * 1024 * 1024, "chunk_size 由服务端给: {v}");
    let (temp, subdir) = (
        d["resource_temp_base_name"].as_str().unwrap().to_string(),
        d["group_sub_dir"].as_str().unwrap().to_string(),
    );
    let (first, second) = png.split_at(30);
    let (st, v) = post_form(
        &c,
        api("/attachments/upload/chunk"),
        Some(&admin),
        chunk_form(&temp, "png", &subdir, &hash, 1, 2, first.to_vec(), "logo.png", &[]),
    )
    .await;
    assert_eq!(st, 200, "第一片: {v}");
    assert!(v["data"]["attachment"].is_null(), "中间片不能出行: {v}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachment").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0, "没传完之前不动库");

    let (st, v) = post_form(
        &c,
        api("/attachments/upload/chunk"),
        Some(&admin),
        chunk_form(&temp, "png", &subdir, &hash, 2, 2, second.to_vec(), "logo.png", &[]),
    )
    .await;
    assert_eq!(st, 200, "末片: {v}");
    let row = v["data"]["attachment"].clone();
    let aid = common::as_id(&row["id"]);
    assert_eq!(row["name"], "logo.png", "原始文件名落库（下载时用它）: {v}");
    assert_eq!(row["ext"], "png", "{v}");
    assert_eq!(row["mime"], "image/png", "Content-Type 由扩展名定（与插件同一张表）: {v}");
    assert_eq!(row["size"], json!(png.len()), "size 按落盘字节实测: {v}");
    assert_eq!(row["sha256"], PNG_SHA256, "sha256 是服务端算的十六进制: {v}");
    assert_eq!(row["admin_id"], json!(admin_id), "上传人对外是短串: {v}");
    assert_eq!(row["entity_type"], "", "没传关联字段就是未绑定: {v}");
    assert_eq!(row["entity_id"], json!(""), "未绑定 = 空串（不是 0 的短串）: {v}");
    assert_eq!(row["is_public"], json!(false), "未绑定的附件不公开: {v}");

    // 库里的行 + 盘上的文件（路径三段式、内容一致）
    let (name, ext, saved, sha, uploader): (String, String, String, String, i64) = sqlx::query_as(
        "SELECT name, ext, stored_path, sha256, admin_id FROM attachment WHERE id = ?",
    )
    .bind(common::dec_id(&aid))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((name.as_str(), ext.as_str()), ("logo.png", "png"));
    assert_eq!(sha, PNG_SHA256, "库里的摘要与响应一致");
    assert_eq!(uploader, admin_num);
    assert!(saved.starts_with("attach_"), "落盘在 attach 分组: {saved}");
    assert_eq!(saved, format!("attach_{subdir}_{hash}.png"), "saved_path 三段式: {saved}");
    assert_eq!(std::fs::read(attach_file(&saved)).unwrap(), png, "成品落盘且内容一致");

    // ── 3. 下载：字节 + 头部；未登录 401；不存在的 404 ───────────────
    let r = c
        .get(api(&format!("/attachments/{aid}")))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "登录后能下载");
    assert_eq!(r.headers()["content-type"], "image/png", "Content-Type 用落库的 mime");
    let disp = r.headers()["content-disposition"].to_str().unwrap().to_string();
    assert!(disp.contains("attachment; filename=\"logo.png\""), "带原文件名: {disp}");
    assert_eq!(r.bytes().await.unwrap().as_ref(), png.as_slice(), "字节与上传一致");

    let r = c.get(api(&format!("/attachments/{aid}"))).send().await.unwrap();
    assert_eq!(r.status(), 401, "下载要登录（<img> 那条公开路是头像专属，附件不给）");

    let (st, v) = call(&c, Method::GET, api(&format!("/attachments/{}", common::enc_id(999))), Some(&admin), None).await;
    assert_eq!(st, 404, "行不存在 → 404: {v}");

    // 路径穿越：id 是短串（解不出 → 400）；库里的路径被改成穿越形态 → 插件校验拦下 → 404
    let r = c.get(api("/attachments/..%2f..%2fetc%2fpasswd")).bearer_auth(&admin).send().await.unwrap();
    assert_eq!(r.status(), 400, "非法 id 是 400 不是 500");
    sqlx::query("UPDATE attachment SET stored_path = ? WHERE id = ?")
        .bind("attach_202610_../../etc/passwd")
        .bind(common::dec_id(&aid))
        .execute(&pool)
        .await
        .unwrap();
    let (st, v) = call(&c, Method::GET, api(&format!("/attachments/{aid}")), Some(&admin), None).await;
    assert_eq!(st, 404, "穿越形态的 stored_path 读不出东西（插件按段校验）: {v}");
    sqlx::query("UPDATE attachment SET stored_path = ? WHERE id = ?")
        .bind(&saved)
        .bind(common::dec_id(&aid))
        .execute(&pool)
        .await
        .unwrap();

    // ── 4. 数据权限负控：别的角色拿不到非公开附件 ─────────────────────
    let (st, v) = call(&c, Method::GET, api("/menus/tree"), Some(&admin), None).await;
    assert_eq!(st, 200, "菜单树: {v}");
    let menu_ids: Vec<String> = ["system:attach:list", "system:attach:add", "system:attach:edit", "system:attach:remove"]
        .iter()
        .map(|p| find_perm(&v["data"], p))
        .collect();
    assert_eq!(menu_ids.len(), 4, "种子里有附件中心四个权限码: {v}");
    let (st, v) = call(
        &c,
        Method::POST,
        api("/roles"),
        Some(&admin),
        Some(json!({"name": "附件测试角色", "code": "att_role", "data_scope": 4, "status": 1, "remark": ""})),
    )
    .await;
    assert_eq!(st, 200, "建角色: {v}");
    let role_id = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/roles/{role_id}/menus")),
        Some(&admin),
        Some(json!({ "menu_ids": menu_ids })),
    )
    .await;
    assert_eq!(st, 200, "挂权限: {v}");
    let (st, v) = call(
        &c,
        Method::POST,
        api("/admins"),
        Some(&admin),
        Some(json!({"username": "att_u", "password": "att_u12345", "nickname": "附件用户", "role_ids": [role_id]})),
    )
    .await;
    assert_eq!(st, 200, "建用户: {v}");
    let other = common::login(&base, "att_u", "att_u12345").await.expect("受限用户登录");

    let (st, v) = call(&c, Method::GET, api(&format!("/attachments/{aid}")), Some(&other), None).await;
    assert_eq!(st, 403, "仅本人范围拿不到别人传的非公开附件（负控）: {v}");
    assert_eq!(v["msg"], "超出你的数据权限范围", "越权文案: {v}");
    let (st, v) = call(&c, Method::GET, api("/attachments"), Some(&other), None).await;
    assert_eq!(st, 200, "列表能进（有 list 码）: {v}");
    assert_eq!(v["data"]["total"], json!(0), "别人的附件在列表里也看不见: {v}");
    let (st, v) = call(&c, Method::DELETE, api(&format!("/attachments/{aid}")), Some(&other), None).await;
    assert_eq!(st, 403, "越权删除同样被拒: {v}");

    let mine = upload_attach(&c, &base, &other, "mine.txt", b"hello attach", &[]).await;
    let mine_id = common::as_id(&mine["attachment"]["id"]);
    let (st, v) = call(&c, Method::GET, api(&format!("/attachments/{mine_id}")), Some(&other), None).await;
    assert_eq!(st, 200, "自己传的自己能下（403 不是一刀切）: {v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/attachments/{mine_id}")), Some(&admin), None).await;
    assert_eq!(st, 200, "超管全量: {v}");

    // ── 4b. add 码负控：没有 system:attach:add 的角色连上传口都进不去 ─────────
    // 上传是唯一「往磁盘写东西」的入口，此前只有登录校验（全仓唯一缺 add 的模块）。
    let (st, v) = call(
        &c,
        Method::POST,
        api("/roles"),
        Some(&admin),
        Some(json!({"name": "无附件权限角色", "code": "att_none", "data_scope": 1, "status": 1, "remark": ""})),
    )
    .await;
    assert_eq!(st, 200, "建无权限角色: {v}");
    let none_role_id = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::POST,
        api("/admins"),
        Some(&admin),
        Some(json!({"username": "att_none_u", "password": "att_none12345", "nickname": "无权限用户", "role_ids": [none_role_id]})),
    )
    .await;
    assert_eq!(st, 200, "建无权限用户: {v}");
    let none_user = common::login(&base, "att_none_u", "att_none12345").await.expect("无权限用户登录");

    let disk_before = tree_files();
    let (st, v) = post_form(
        &c,
        api("/attachments/upload/preprocess"),
        Some(&none_user),
        preprocess_form("nope.png", png.len(), &hash, &[]),
    )
    .await;
    assert_eq!(st, 403, "没有 add 码：preprocess 403: {v}");
    let (st, v) = post_form(
        &c,
        api("/attachments/upload/chunk"),
        Some(&none_user),
        chunk_form("nope-tmp", "png", "202610", &hash, 1, 1, png.clone(), "nope.png", &[]),
    )
    .await;
    assert_eq!(st, 403, "没有 add 码：chunk 403: {v}");
    assert_eq!(tree_files(), disk_before, "403 要在碰磁盘之前发生（零落盘）");

    // ── 5. 公告配图：绑定已发布公告 = 公开（对全体登录用户可下）────────
    let (st, v) = call(
        &c,
        Method::POST,
        api("/notices"),
        Some(&admin),
        Some(json!({"title": "附件公开规则", "content": "配图测试", "status": 0})),
    )
    .await;
    assert_eq!(st, 200, "建草稿公告: {v}");
    let notice_id = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/attachments/{aid}")),
        Some(&admin),
        Some(json!({"entity_type": "notice", "entity_id": notice_id, "sort": 3})),
    )
    .await;
    assert_eq!(st, 200, "绑定草稿公告: {v}");
    let (st, v) = call(&c, Method::GET, api("/attachments"), Some(&admin), None).await;
    assert_eq!(st, 200, "列表: {v}");
    let bound = v["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == json!(aid))
        .expect("列表里能按 id 找到刚关联的那条");
    assert_eq!(v["data"]["list"][0]["id"], json!(mine_id), "倒序：最新的在最前: {v}");
    assert_eq!(bound["entity_type"], "notice");
    assert_eq!(bound["sort"], json!(3), "排序落库: {v}");
    assert_eq!(bound["entity_label"], "附件公开规则", "列表回显关联对象标题: {v}");
    assert_eq!(bound["is_public"], json!(false), "草稿公告的配图不公开: {v}");

    // 按对象收窄：公告编辑页只拉「这篇公告的配图」（列表本身受数据权限约束，是超管才看得到全部）
    let (st, v) = call(
        &c,
        Method::GET,
        api(&format!("/attachments?entity_type=notice&entity_id={notice_id}")),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(st, 200, "按关联对象筛: {v}");
    assert_eq!(v["data"]["total"], json!(1), "只回这篇公告的配图: {v}");
    assert_eq!(v["data"]["list"][0]["id"], json!(aid));
    let (st, _) = call(
        &c,
        Method::GET,
        api("/attachments?entity_id=not-a-hashid"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(st, 400, "非法 entity_id 短串要 400，不能静默当成「没有」");

    let (st, v) = call(&c, Method::GET, api(&format!("/attachments/{aid}")), Some(&other), None).await;
    assert_eq!(st, 403, "草稿公告的附件仍是非公开（负控）: {v}");

    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/notices/{notice_id}")),
        Some(&admin),
        Some(json!({"title": "附件公开规则", "content": "配图测试", "status": 1})),
    )
    .await;
    assert_eq!(st, 200, "发布公告: {v}");
    let r = c
        .get(api(&format!("/attachments/{aid}")))
        .bearer_auth(&other)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "已发布公告的附件对全体登录用户可下");
    assert_eq!(r.bytes().await.unwrap().as_ref(), png.as_slice());
    let (st, v) = call(&c, Method::GET, api("/attachments"), Some(&other), None).await;
    assert_eq!(st, 200, "列表: {v}");
    assert_eq!(v["data"]["total"], json!(1), "受限用户只看得到自己那份: {v}");
    assert_eq!(v["data"]["list"][0]["id"], json!(mine_id));
    assert!(
        !v["data"]["list"].as_array().unwrap().iter().any(|r| r["id"] == json!(aid)),
        "公开只放开下载，不放开列表（别人的附件仍看不见）: {v}"
    );

    // 解绑 → 又变回非公开（证明「公开」完全由「绑定已发布公告」驱动）
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/attachments/{aid}")),
        Some(&admin),
        Some(json!({"entity_type": "", "entity_id": "", "sort": 0})),
    )
    .await;
    assert_eq!(st, 200, "解绑: {v}");
    let (st, v) = call(&c, Method::GET, api(&format!("/attachments/{aid}")), Some(&other), None).await;
    assert_eq!(st, 403, "解绑后立刻收回: {v}");

    // 绑定参数校验：只认 notice；公告不存在 → 404
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/attachments/{aid}")),
        Some(&admin),
        Some(json!({"entity_type": "evil", "entity_id": notice_id, "sort": 0})),
    )
    .await;
    assert_eq!(st, 400, "未知的关联类型要拒: {v}");
    let (st, v) = call(
        &c,
        Method::PUT,
        api(&format!("/attachments/{aid}")),
        Some(&admin),
        Some(json!({"entity_type": "notice", "entity_id": common::enc_id(987654), "sort": 0})),
    )
    .await;
    assert_eq!(st, 404, "公告不存在不给出幽灵关联: {v}");

    // ── 6. 删除 + 引用计数（同一份内容共用同一个文件）─────────────────
    let again = upload_attach(&c, &base, &admin, "logo-copy.png", &png, &[]).await;
    let copy_id = common::as_id(&again["attachment"]["id"]);
    let copy_saved = sqlx::query_scalar::<_, String>("SELECT stored_path FROM attachment WHERE id = ?")
        .bind(common::dec_id(&copy_id))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(copy_saved, saved, "同内容重传落在同一个成品文件上（秒传/同 md5）");
    assert_ne!(copy_id, aid, "两次上传是两行（各自的关联与删除互不影响）");

    let (st, v) = call(&c, Method::DELETE, api(&format!("/attachments/{aid}")), Some(&admin), None).await;
    assert_eq!(st, 200, "删除: {v}");
    assert!(attach_file(&saved).exists(), "还有别的行引用同一文件 → 文件必须留着");
    let (st, v) = call(&c, Method::GET, api(&format!("/attachments/{aid}")), Some(&admin), None).await;
    assert_eq!(st, 404, "删掉的附件下载 404: {v}");

    let (st, v) = call(&c, Method::DELETE, api(&format!("/attachments/{copy_id}")), Some(&admin), None).await;
    assert_eq!(st, 200, "删最后一份引用: {v}");
    // 批次 E S2：删行**不再顺手删文件**。接口里删文件与「数还有没有别的行引用」分开跑，
    // 两条之间有人秒传重传同一份内容（同 md5 → 同一个成品路径）就会把新行指着的文件删掉。
    // 文件改由孤儿清理收（第 9 节的断言），代价是磁盘要到下一轮清理才回收。
    assert!(attach_file(&saved).exists(), "S2：删行不删文件（竞态窗口），文件交给孤儿清理");

    // ── 7. 类型 / 大小 / 配额闸门 ──────────────────────────────────
    for bad in ["evil.php", "shell.exe", "page.svg"] {
        let (st, v) = post_form(
            &c,
            api("/attachments/upload/preprocess"),
            Some(&admin),
            preprocess_form(bad, 10, &aetherupload::md5_hex(&png), &[]),
        )
        .await;
        assert_eq!(st, 415, "白名单外的扩展名（{bad}）: {v}");
    }
    let (st, v) = post_form(
        &c,
        api("/attachments/upload/preprocess"),
        Some(&admin),
        preprocess_form("huge.png", 11 * 1024 * 1024, &aetherupload::md5_hex(&png), &[]),
    )
    .await;
    assert_eq!(st, 400, "超过 10MB 单文件上限: {v}");
    assert_eq!(v["err"], "attach.too_large", "稳定码给前端译: {v}");

    // 配额（每人总量上限）：换一个 quota_mb=1 的进程，先把 200KB 传进去再要 900KB
    let (_quota_server, qbase) = common::start_server_conf(|c| c.replace("quota_mb = 50", "quota_mb = 1")).await;
    let qlogin = common::login(&qbase, "admin", "admin123").await.expect("配额进程登录");
    let jpeg = jpeg_bytes();
    let kept = upload_attach(&c, &qbase, &qlogin, "small.jpg", &jpeg, &[]).await;
    assert!(kept["attachment"]["id"].is_string(), "配额内能传: {kept}");
    let (st, v) = post_form(
        &c,
        format!("{qbase}/api/v1/attachments/upload/preprocess"),
        Some(&qlogin),
        preprocess_form("next.jpg", 900 * 1024, &aetherupload::md5_hex(&png), &[]),
    )
    .await;
    assert_eq!(st, 400, "0.2MB 已用 + 0.9MB 新文件 > 1MB 配额: {v}");
    assert_eq!(v["err"], "attach.quota_exceeded", "配额码给前端译: {v}");

    // 不诚实路径：预检声明 resource_size=1（过得去），落盘实测量 ~900KB ——
    // 预检只看声明值，唯一拦得住的是落库前按实测 size 的复查。
    let mut big = jpeg_bytes();
    big.extend(std::iter::repeat(9u8).take(700 * 1024));
    let big_hash = aetherupload::md5_hex(&big);
    let (st, v) = post_form(
        &c,
        format!("{qbase}/api/v1/attachments/upload/preprocess"),
        Some(&qlogin),
        preprocess_form("sneaky.jpg", 1, &big_hash, &[]),
    )
    .await;
    assert_eq!(st, 200, "声明 1 字节时预检拦不住（正是要补的那道缝）: {v}");
    let d = &v["data"];
    let (temp, ext, subdir) = (
        d["resource_temp_base_name"].as_str().unwrap().to_string(),
        d["resource_ext"].as_str().unwrap().to_string(),
        d["group_sub_dir"].as_str().unwrap().to_string(),
    );
    let chunk_size = d["chunk_size"].as_u64().unwrap() as usize;
    let total = big.len().div_ceil(chunk_size);
    let (mut last_st, mut last_v) = (0, json!(null));
    for idx in 1..=total {
        let start = (idx - 1) * chunk_size;
        let end = (start + chunk_size).min(big.len());
        let (st, v) = post_form(
            &c,
            format!("{qbase}/api/v1/attachments/upload/chunk"),
            Some(&qlogin),
            chunk_form(&temp, &ext, &subdir, &big_hash, idx, total, big[start..end].to_vec(), "sneaky.jpg", &[]),
        )
        .await;
        (last_st, last_v) = (st, v);
        if st != 200 {
            break;
        }
    }
    assert_eq!(last_st, 400, "末片按实测 size 复查配额: {last_v}");
    assert_eq!(last_v["err"], "attach.quota_exceeded", "稳定码: {last_v}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachment WHERE name = 'sneaky.jpg'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "超配额的成品不落行");
    let sneaky = attach_file(&format!("attach_{subdir}_{big_hash}.jpg"));
    assert!(sneaky.exists(), "成品已组装落盘（没行引用，等孤儿清理兜）: {}", sneaky.display());

    // ── 8. 审计留痕（模块 attach）──────────────────────────────────
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE module = 'attach'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(n > 0, "上传/删除都要进操作日志: {n}");
    for action in ["上传附件", "删除附件", "关联附件"] {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE module = 'attach' AND action = ?")
            .bind(action)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(n > 0, "有「{action}」的审计行: {n}");
    }

    orphan_cleanup_only_collects_unreferenced_files(&c, &base, &api, &pool, &admin, &saved, &sneaky).await;
    let _ = std::fs::remove_dir_all("target/test-uploads/attach");
}
