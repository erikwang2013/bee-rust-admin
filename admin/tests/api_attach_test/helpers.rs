// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 附件集成测试的本地构造（F3 拆分：主文件只留用例）。

use crate::common;
use reqwest::Method;
use serde_json::Value;

/// 1x1 PNG（70 字节，真实文件字节）。
pub(super) const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
/// 上面这份 PNG 的 sha256（**测试侧独立算好的常量**，不是拿被测代码的输出当期望）。
pub(super) const PNG_SHA256: &str = "c414cd0e204de974f73753c7e28d7638e7b3691bb8b1a2bab6b25bb7fed7ce77";

pub(super) fn png_bytes() -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(PNG_B64).unwrap()
}

/// 「假 jpg」：插件按魔数认类型，这里只用来验证扩展名闸门（内容不参与断言）。
pub(super) fn jpeg_bytes() -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.extend_from_slice(&[7u8; 200 * 1024]);
    bytes
}

pub(super) async fn call(
    c: &reqwest::Client,
    method: Method,
    url: String,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let mut rb = c.request(method, url);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    let r = rb.send().await.expect("请求发送失败");
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

pub(super) async fn post_form(
    c: &reqwest::Client,
    url: String,
    token: Option<&str>,
    form: reqwest::multipart::Form,
) -> (u16, Value) {
    let mut rb = c.post(url).multipart(form);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    let r = rb.send().await.expect("请求发送失败");
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

/// 上传的额外交付字段（多态关联）：`entity_type` / `entity_id`(短串) / `sort`。
pub(super) type Extra<'a> = &'a [(&'a str, &'a str)];

pub(super) fn preprocess_form(name: &str, size: usize, hash: &str, extra: Extra<'_>) -> reqwest::multipart::Form {
    let mut f = reqwest::multipart::Form::new()
        .text("resource_name", name.to_string())
        .text("resource_size", size.to_string())
        .text("resource_hash", hash.to_string())
        .text("group", "attach".to_string())
        .text("name", name.to_string())
        .text("locale", "zh".to_string());
    for (k, v) in extra {
        f = f.text((*k).to_string(), (*v).to_string());
    }
    f
}

#[allow(clippy::too_many_arguments)]
pub(super) fn chunk_form(
    temp_base: &str,
    ext: &str,
    subdir: &str,
    hash: &str,
    index: usize,
    total: usize,
    bytes: Vec<u8>,
    name: &str,
    extra: Extra<'_>,
) -> reqwest::multipart::Form {
    let mut f = reqwest::multipart::Form::new()
        .text("chunk_total", total.to_string())
        .text("chunk_index", index.to_string())
        .text("resource_temp_basename", temp_base.to_string())
        .text("resource_ext", ext.to_string())
        .text("group_subdir", subdir.to_string())
        .text("resource_hash", hash.to_string())
        .text("group", "attach".to_string())
        .text("name", name.to_string())
        .text("locale", "zh".to_string())
        .part(
            "resource_chunk",
            reqwest::multipart::Part::bytes(bytes).file_name("blob"),
        );
    for (k, v) in extra {
        f = f.text((*k).to_string(), (*v).to_string());
    }
    f
}

/// 走完整流程传一份附件（preprocess + 按服务端 chunk_size 切片），返回末片的 `data`。
pub(super) async fn upload_attach(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    bytes: &[u8],
    extra: Extra<'_>,
) -> Value {
    let url = |p: &str| format!("{base}/api/v1{p}");
    let hash = aetherupload::md5_hex(bytes);
    let (st, v) = post_form(
        c,
        url("/attachments/upload/preprocess"),
        Some(token),
        preprocess_form(name, bytes.len(), &hash, extra),
    )
    .await;
    assert_eq!(st, 200, "preprocess({name}): {v}");
    let head = v["data"].clone();
    if !head["saved_path"].as_str().unwrap_or("").is_empty() {
        return head; // 秒传命中
    }
    let chunk_size = head["chunk_size"].as_u64().expect("chunk_size") as usize;
    let total = bytes.len().div_ceil(chunk_size);
    let (temp, ext, subdir) = (
        head["resource_temp_base_name"].as_str().unwrap().to_string(),
        head["resource_ext"].as_str().unwrap().to_string(),
        head["group_sub_dir"].as_str().unwrap().to_string(),
    );
    let mut last = head;
    for idx in 1..=total {
        let start = (idx - 1) * chunk_size;
        let end = (start + chunk_size).min(bytes.len());
        let (st, v) = post_form(
            c,
            url("/attachments/upload/chunk"),
            Some(token),
            chunk_form(&temp, &ext, &subdir, &hash, idx, total, bytes[start..end].to_vec(), name, extra),
        )
        .await;
        assert_eq!(st, 200, "{name} 第 {idx}/{total} 片: {v}");
        last = v["data"].clone();
    }
    last
}

/// 成品落盘路径：`{upload_dir}/attach/{子目录}/{文件名}`（saved_path 三段式）。
pub(super) fn attach_file(saved_path: &str) -> std::path::PathBuf {
    let mut parts = saved_path.splitn(3, '_');
    let (_, subdir, name) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
    std::path::Path::new("target/test-uploads/attach").join(subdir).join(name)
}

/// 附件组目录下的全部文件（递归、排序）——「零落盘」类断言的前后快照。
/// 只走 attach 子树：上传根下还有 backups（db_backup 任务随时可能落一份）等别的目录，
/// 全根快照会被后台任务搅出假失败。
pub(super) fn tree_files() -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else {
                    out.push(p);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(std::path::Path::new("target/test-uploads/attach"), &mut out);
    out.sort();
    out
}

/// 把文件的 mtime 拨回 `mins` 分钟前（S2 的保护期只看 mtime：新的不碰、老的才收，
/// 所以「最终会被收走」这半条得先把文件拨老才测得出来）。
pub(super) fn backdate(path: &std::path::Path, mins: u64) {
    let t = std::time::SystemTime::now() - std::time::Duration::from_secs(mins * 60);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap_or_else(|e| panic!("拨 mtime 打不开 {}: {e}", path.display()))
        .set_modified(t)
        .unwrap_or_else(|e| panic!("拨 mtime 失败 {}: {e}", path.display()));
}

/// 菜单树里某个权限码对应的 id（造受限角色用）。
pub(super) fn find_perm(tree: &Value, perm: &str) -> String {
    // `tree` 是顶层节点**数组**：从这里展开，别把整个数组当一个节点找（它没有 perm 字段）
    let mut stack: Vec<&Value> = tree.as_array().map(|a| a.iter().collect()).unwrap_or_default();
    while let Some(node) = stack.pop() {
        if node["perm"] == perm {
            return common::as_id(&node["id"]);
        }
        if let Some(children) = node["children"].as_array() {
            stack.extend(children.iter());
        }
    }
    panic!("菜单树里没有 {perm}");
}

/// 第 9 节原样搬出（F3 拆行数：断言零改动）。孤儿清理只收无引用成品，
/// 保护期 / 白名单 / 活引用都不碰；旧执行者与接口删除留下的文件都由它兜底。
pub(super) async fn orphan_cleanup_only_collects_unreferenced_files(
    c: &reqwest::Client,
    base: &str,
    api: &impl Fn(&str) -> String,
    pool: &sqlx::MySqlPool,
    admin: &str,
    saved: &str,
    sneaky: &std::path::Path,
) {
    // ── 9. 孤儿清理（attach_orphan_clean）：没人引用的成品要清掉，活着的不能误删 ──
    // 内容多一个字节：附件是按 md5 内容寻址的，直接用 `png` 会落在第 6 节那个已经
    // 没人引用的成品路径上（那份文件本轮要被收走，两边的断言就打架了）。
    let mut live_bytes = png_bytes();
    live_bytes.extend_from_slice(b"live");
    let live = upload_attach(c, base, admin, "clean.png", &live_bytes, &[]).await;
    let live_saved = live["saved_path"].as_str().expect("成品 saved_path").to_string();
    assert!(attach_file(&live_saved).exists(), "刚传的附件在盘上: {live}");
    let dir = std::path::Path::new("target/test-uploads/attach/202610");
    std::fs::create_dir_all(dir).unwrap();
    // 白名单内、没人引用 = 孤儿（落盘成功却没落库的那条缝，只有这个任务兜得住）
    let orphan = dir.join("orphan-deadbeef.png");
    std::fs::write(&orphan, b"stray").unwrap();
    // 插件写到一半的临时文件（扩展名不在白名单）：清理任务不许碰
    let part = dir.join("half.png.part");
    std::fs::write(&part, b"partial").unwrap();
    // 上一节被配额拒掉的成品也躺在这儿（没行引用）—— 它就是「落库前复查」那条缝的兜底对象
    assert!(sneaky.exists(), "被拒的成品在盘上等清理: {}", sneaky.display());

    let (st, v) = call(&c, Method::GET, api("/jobs"), Some(admin), None).await;
    assert_eq!(st, 200, "任务列表: {v}");
    let job = v["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["code"] == "attach_orphan_clean")
        .expect("任务由代码注册表补齐");
    let job_id = common::as_id(&job["id"]);
    let mark = common::last_log_id(pool, "attach_orphan_clean").await;
    let (st, v) = call(&c, Method::POST, api(&format!("/jobs/{job_id}/run")), Some(admin), None).await;
    assert_eq!(st, 200, "手动触发清理: {v}");
    let (status, msg, _) = common::wait_job_log(pool, "attach_orphan_clean", mark).await;
    assert_eq!(status, 1, "清理任务执行成功: {msg}");
    // 第一轮：这三个都是**刚写下**的（S2 保护期内）—— 清理任务必须一个都不碰。
    // 保护期挡的是「上传末片落盘 → INSERT 之间」那条缝：撞进去就是删活文件。
    assert!(orphan.exists(), "保护期内的新孤儿这轮不碰（下一轮才收）: {msg}");
    assert!(sneaky.exists(), "同上（被配额拒掉的成品也只是晚一轮收）: {msg}");
    assert!(attach_file(&saved).exists(), "第 6 节删掉的那份同理，晚一轮: {msg}");
    assert!(part.exists(), "非白名单扩展名（插件临时文件）不碰: {msg}");
    assert!(attach_file(&live_saved).exists(), "库里有引用的文件绝不能删（误删=丢数据）: {msg}");

    // 第二轮：把它们拨老到保护期外（6 分钟 > 5 分钟），该收的这次都要收走
    backdate(&orphan, 6);
    backdate(&sneaky, 6);
    backdate(&attach_file(&saved), 6);
    let mark = common::last_log_id(pool, "attach_orphan_clean").await;
    let (st, v) = call(&c, Method::POST, api(&format!("/jobs/{job_id}/run")), Some(admin), None).await;
    assert_eq!(st, 200, "手动触发清理（第二轮）: {v}");
    let (status, msg, _) = common::wait_job_log(pool, "attach_orphan_clean", mark).await;
    assert_eq!(status, 1, "清理任务执行成功: {msg}");
    assert!(!orphan.exists(), "没人引用的成品被清掉: {msg}");
    assert!(!sneaky.exists(), "被配额拒掉的成品同样收走（I1 的兜底路径真的接得住）: {msg}");
    assert!(
        !attach_file(&saved).exists(),
        "S2 的落点：接口删行之后，文件最终被孤儿任务收走: {msg}"
    );
    assert!(part.exists(), "非白名单扩展名（插件临时文件）还是不碰: {msg}");
    assert!(attach_file(&live_saved).exists(), "库里有引用的文件绝不能删（误删=丢数据）: {msg}");
}
