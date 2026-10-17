// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use super::*;

/// 一次最多导入的数据行数。
pub(crate) const MAX_ROWS: usize = 5000;

#[derive(Deserialize)]
pub struct KindQuery {
    /// `admin` / `dept` / `dict-item`。
    pub kind: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Admin,
    Dept,
    DictItem,
}

impl Kind {
    pub(crate) fn parse(s: &str) -> Result<Self, ApiError> {
        match s {
            "admin" => Ok(Kind::Admin),
            "dept" => Ok(Kind::Dept),
            "dict-item" => Ok(Kind::DictItem),
            other => Err(ApiError::BadRequest(format!(
                "不认识的导入类型 {other:?}（可选 admin / dept / dict-item）"
            ))),
        }
    }

    /// 回给前端的种类名（与 query 参数同形），也是模板文件名的前缀。
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Admin => "admin",
            Kind::Dept => "dept",
            Kind::DictItem => "dict-item",
        }
    }

    /// 复用各模块的新增权限码：能新增就能导入（不新增权限码，批次 B 台账有 Ruling）。
    pub(crate) fn perm(self) -> &'static str {
        match self {
            Kind::Admin => "system:admin:add",
            Kind::Dept => "system:dept:add",
            Kind::DictItem => "system:dict:add",
        }
    }

    /// 表头 = 解析时逐字比对的列名（模板、前端提示、usage 文档都按这份走）。
    pub(crate) fn headers(self) -> &'static [&'static str] {
        match self {
            Kind::Admin => &["用户名", "密码", "昵称", "邮箱", "手机号", "部门", "角色", "状态", "备注"],
            Kind::Dept => &["部门名称", "上级部门", "负责人", "联系电话", "排序", "状态"],
            Kind::DictItem => &["类型编码", "标签", "值", "排序", "状态", "备注"],
        }
    }
}

/// 校验通过、待落库的一行。`role_ids` 只有管理员行有（关联表另插）。
pub(crate) enum Item {
    Admin { row: Admin, role_ids: Vec<i64> },
    Dept(Dept),
    Dict(DictItem),
}

/// 逐行报告：`msg` 只在坏行非空。
pub(crate) struct Checked {
    line: u64,
    item: Option<Item>,
    msg: String,
}

#[apidoc::title("下载导入模板")]
#[apidoc::desc("只回表头（CSV + BOM，Excel 直接可编辑）。列名不能改：导入时逐字比对表头，改过会整份拒绝。数据行请照模板列序填，空行会被忽略。")]
#[apidoc::url("/api/v1/imports/template")]
#[apidoc::method("GET")]
#[apidoc::tag("数据导入")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "kind", ty = "string", required, desc = "导入类型：admin 管理员 / dept 部门 / dict-item 字典项")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("403")]
#[apidoc::returned(name = "data", ty = "file", desc = "CSV 附件（text/csv; charset=utf-8）")]
pub async fn template(
    auth: Auth,
    Query(q): Query<KindQuery>,
) -> Result<Response, ApiError> {
    let kind = Kind::parse(&q.kind)?;
    auth.require(kind.perm())?;
    let cells: Vec<String> = kind.headers().iter().map(|h| h.to_string()).collect();
    let body = format!("{}{}", BOM, csv_row(&cells));
    let name = format!("{}_import_template.csv", kind.name());
    Ok((csv_headers(&name), body).into_response())
}

#[apidoc::title("CSV 导入")]
#[apidoc::desc("multipart 表单（字段名 file）。先全量校验再落库：有一行不合规就整批不写（created=0），逐行报告给行号与原因，好行也是 ok=true。管理员行的「部门」「角色」按名称定位（角色多个用 | 分隔，与导出同分隔符）；部门行的「上级部门」必须是库里已有的部门。")]
#[apidoc::url("/api/v1/imports")]
#[apidoc::method("POST")]
#[apidoc::tag("数据导入")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "kind", ty = "string", required, desc = "导入类型：admin 管理员 / dept 部门 / dict-item 字典项")]
#[apidoc::param(name = "file", ty = "file", required, desc = "CSV 文件（UTF-8 / CSV UTF-8，带不带 BOM 都认）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("403")]
#[apidoc::returned(name = "data", ty = "object", desc = "导入报告", children = [
        {name = "kind", ty = "string", desc = "导入类型"},
        {name = "total", ty = "int", desc = "数据行数（不含表头与空行）"},
        {name = "created", ty = "int", desc = "实际写入条数：全对 = total，有坏行 = 0"},
        {name = "rows", ty = "array", desc = "逐行报告", children = [
            {name = "line", ty = "int", desc = "文件行号（表头是第 1 行）"},
            {name = "ok", ty = "bool", desc = "该行是否通过校验"},
            {name = "msg", ty = "string", desc = "不通过的原因（通过时为空串）"},
        ]},
    ])]
pub async fn import(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<KindQuery>,
    mut multipart: AppMultipart,
) -> Result<Json<JsonVal>, ApiError> {
    let kind = Kind::parse(&q.kind)?;
    auth.require(kind.perm())?;
    let bytes = read_file(&mut multipart.0).await?;
    let text = decode(&bytes)?;

    let checked = check_all(&state, &auth, kind, &text).await?;
    let total = checked.len();
    let created = if checked.iter().any(|c| c.item.is_none()) {
        0 // 有坏行：一条都不写（报告里已逐行说明）
    } else {
        write_all(&state, checked.iter().filter_map(|c| c.item.as_ref())).await?
    };
    let rows: Vec<JsonVal> = checked
        .iter()
        .map(|c| json!({ "line": c.line, "ok": c.item.is_some(), "msg": c.msg }))
        .collect();
    Ok(ok(json!({ "kind": kind.name(), "total": total, "created": created, "rows": rows })))
}

/// 取表单里的 `file` 字段（字段名与前端逐字一致）。
pub(crate) async fn read_file(mp: &mut Multipart) -> Result<Vec<u8>, ApiError> {
    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("上传表单解析失败: {e}")))?
    {
        if field.name() == Some("file") {
            return field
                .bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| ApiError::BadRequest(format!("读取上传文件失败: {e}")));
        }
    }
    Err(ApiError::BadRequest("表单里没有 file 字段".into()))
}

/// 字节 → 文本：去 BOM + 认 UTF-8。
pub(crate) fn decode(bytes: &[u8]) -> Result<String, ApiError> {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(body.to_vec())
        .map_err(|_| ApiError::BadRequest("文件不是 UTF-8 编码（Excel 请用「CSV UTF-8」另存）".into()))
}

/// 校验全表：表头 → 预载参照数据 → 逐行。返回逐行报告（好行带待落库实体）。
pub(crate) async fn check_all(
    state: &AppState,
    auth: &Auth,
    kind: Kind,
    text: &str,
) -> Result<Vec<Checked>, ApiError> {
    // flexible：列数不对的行也读得出来，好在报告里点名（而不是整份文件解析失败）
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(text.as_bytes());
    let head = rdr
        .headers()
        .map_err(|e| ApiError::BadRequest(format!("解析表头失败: {e}")))?
        .clone();
    let want = kind.headers();
    let same = head.len() == want.len()
        && want.iter().zip(head.iter()).all(|(w, got)| w.trim() == got.trim());
    if !same {
        return Err(ApiError::BadRequest(format!(
            "表头与模板不一致，应为: {}",
            want.join(",")
        )));
    }

    let mut ctx = Ctx::load(state, kind).await?;
    let mut out: Vec<Checked> = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| ApiError::BadRequest(format!("CSV 解析失败: {e}")))?;
        if rec.iter().all(|c| c.trim().is_empty()) {
            continue; // 空行（含末尾换行产生的幻影行）
        }
        if out.len() >= MAX_ROWS {
            return Err(ApiError::BadRequest(format!("一次最多导入 {MAX_ROWS} 行")));
        }
        let line = rec.position().map(|p| p.line()).unwrap_or(0);
        let r = match kind {
            Kind::Admin => admin_item(state, auth, &mut ctx, &rec).await,
            Kind::Dept => dept_item(state, &mut ctx, &rec),
            Kind::DictItem => dict_item(state, &mut ctx, &rec),
        };
        match r {
            Ok(item) => out.push(Checked { line, item: Some(item), msg: String::new() }),
            // 内部错误不是这一行的问题：整份冒出去（别伪装成「你的文件写错了」）
            Err(e @ ApiError::Internal(_)) => return Err(e),
            Err(e) => out.push(Checked { line, item: None, msg: reason(&e) }),
        }
    }
    Ok(out)
}

/// 行内报告用的原因文本：取错误的中文 `msg`（`Internal` 的细节只留日志，不外泄）。
pub(crate) fn reason(e: &ApiError) -> String {
    match e {
        ApiError::BadRequest(m) | ApiError::Forbidden(m) | ApiError::NotFoundMsg(m) => m.clone(),
        ApiError::Coded { msg, .. } => msg.clone(),
        ApiError::Unauthorized => "登录已失效，请重新登录".into(),
        ApiError::NotFound => "资源不存在".into(),
        ApiError::Internal(_) => "服务器内部错误".into(),
    }
}

/// 一次预载的参照数据 + 文件内去重集合（行内校验不再查库；5000 行 = 4 次查询）。
pub(crate) struct Ctx {
    pub(crate) depts: Vec<Dept>,
    pub(crate) roles: Vec<Role>,
    /// 库里已有的用户名。
    pub(crate) usernames: HashSet<String>,
    /// 本文件里已出现过的用户名（同文件重复也算「已存在」）。
    pub(crate) seen_users: HashSet<String>,
    pub(crate) dict_codes: HashSet<String>,
    pub(crate) dict_pairs: HashSet<(String, String)>,
    pub(crate) seen_pairs: HashSet<(String, String)>,
}

impl Ctx {
    async fn load(state: &AppState, kind: Kind) -> Result<Self, ApiError> {
        let mut ctx = Ctx {
            depts: Vec::new(),
            roles: Vec::new(),
            usernames: HashSet::new(),
            seen_users: HashSet::new(),
            dict_codes: HashSet::new(),
            dict_pairs: HashSet::new(),
            seen_pairs: HashSet::new(),
        };
        if matches!(kind, Kind::Admin | Kind::Dept) {
            ctx.depts = Dept::query().all(&state.db).await.map_err(ApiError::from)?;
        }
        if kind == Kind::Admin {
            ctx.roles = Role::query().all(&state.db).await.map_err(ApiError::from)?;
            // 只取一列：admin 表可能有几万行，用户名做唯一性预检就够了
            ctx.usernames = state
                .db
                .query("SELECT username FROM admin", &[])
                .await
                .map_err(ApiError::from)?
                .iter()
                .filter_map(|r| r.get("username").and_then(|v| v.as_str()).map(str::to_string))
                .collect();
        }
        if kind == Kind::DictItem {
            ctx.dict_codes = DictType::query()
                .all(&state.db)
                .await
                .map_err(ApiError::from)?
                .into_iter()
                .map(|t| t.code)
                .collect();
            ctx.dict_pairs = state
                .db
                .query("SELECT type_code, value FROM dict_item", &[])
                .await
                .map_err(ApiError::from)?
                .iter()
                .filter_map(|r| {
                    Some((
                        r.get("type_code")?.as_str()?.to_string(),
                        r.get("value")?.as_str()?.to_string(),
                    ))
                })
                .collect();
        }
        Ok(ctx)
    }
}
