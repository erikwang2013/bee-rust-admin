// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 导出中心（项 4）：导出任务的列表 / 发起 / 下载 / 删除。
//!
//! 执行不在请求里——任务落库后由 `crate::export` 的 job（`export_run`）跑，过期由
//! `export_clean` 清。请求路径只负责「登记」「取消」与「取回成品」。
//!
//! 权限口径（两条闸都要过）：
//! - `system:export:list`：能用导出中心（菜单入口码）；
//! - 该 kind 的 `system:<模块>:list`：看得到那类数据（复用各模块的码，不新增）。
//! 且**只碰自己发起的任务**（导出中心是个人工作台）：别人的记录看不见、下不了、删不掉。
use crate::api::{PagingExt, csv, page_size};
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::export;
use crate::hid;
use crate::models::ExportTask;
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use bee_orm::Model;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct ExportQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub kind: Option<String>,
    /// 0 待处理 / 1 执行中 / 2 已完成 / 3 失败 / 4 已取消
    pub status: Option<i8>,
}

#[derive(Deserialize)]
pub struct CreateBody {
    /// `KINDS` 里的注册键（操作日志 audit-log / 登录记录 login-log）
    pub kind: String,
    /// 筛选条件，字段与同步导出的 query 一致（省略 = 全量）
    pub params: Option<Value>,
}

#[apidoc::title("导出任务列表")]
#[apidoc::desc("只列自己发起的导出任务（导出中心是个人工作台，超管也一样）")]
#[apidoc::url("/api/v1/exports")]
#[apidoc::method("GET")]
#[apidoc::tag("导出")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100")]
#[apidoc::query(name = "kind", ty = "string", desc = "导出类型：audit-log / login-log")]
#[apidoc::query(name = "status", ty = "int", desc = "0 待处理 / 1 执行中 / 2 已完成 / 3 失败 / 4 已取消")]
#[apidoc::response_status("200")]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<ExportQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:export:list")?;
    let (page, size) = page_size(q.page, q.size);
    let mut qs = ExportTask::query()
        .filter_eq("admin_id", auth.admin.id)
        .map_err(ApiError::from)?;
    if let Some(k) = q.kind.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_eq("kind", k.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    let (rows, total) = qs
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

#[apidoc::title("发起导出")]
#[apidoc::desc("登记一个导出任务，由定时任务（export_run，默认每分钟）执行；配合列表轮询看状态")]
#[apidoc::url("/api/v1/exports")]
#[apidoc::method("POST")]
#[apidoc::tag("导出")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "新建的任务（id / kind / status）")]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(b): AppJson<CreateBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:export:list")?;
    let kind = export::spec(&b.kind)
        .ok_or_else(|| ApiError::BadRequest(format!("未知导出类型「{}」", b.kind)))?;
    // 看得到那类数据才导得出：复用各模块的列表码，不新增权限码
    auth.require(kind.perm)?;
    let params = b.params.unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Err(ApiError::BadRequest("params 必须是对象".into()));
    }
    // 发起时就把筛选条件校验掉：类型不对当场 400，别等任务跑起来才失败
    (kind.check)(&params).map_err(|e| ApiError::BadRequest(format!("筛选参数不合法: {e}")))?;
    // 列宽 1024（export_task.params）：超了别放过去让 INSERT 炸 500
    let text = params.to_string();
    if text.chars().count() > 1024 {
        return Err(ApiError::too_long("params", 1024));
    }

    let row = ExportTask {
        id: state.next_id()?,
        admin_id: auth.admin.id,
        username: auth.admin.username.clone(),
        kind: kind.code.into(),
        params: text,
        status: 0,
        file_path: String::new(),
        row_count: 0,
        file_size: 0,
        progress_rows: 0,
        msg: String::new(),
        created_at: now(),
        started_at: None,
        heartbeat_at: None,
        finished_at: None,
        expire_at: now() + chrono::Duration::days(state.cfg.export_retain_days),
    };
    row.insert(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(json!({ "id": hid::enc(row.id), "kind": row.kind, "status": row.status })))
}

/// 取回成品。路径不对外当链接发（下载要带 Bearer）：行里存的是绝对路径，
/// 这里直接读文件回附件。
#[apidoc::title("下载导出文件")]
#[apidoc::desc("只给已完成的自己的任务；未完成 400、不是自己的 403、文件没了 404")]
#[apidoc::url("/api/v1/exports/{id}/download")]
#[apidoc::method("GET")]
#[apidoc::tag("导出")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::not_debug]
pub async fn download(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Response, ApiError> {
    auth.require("system:export:list")?;
    let task = own(&state, &auth, &id).await?;
    let kind = export::spec(&task.kind)
        .ok_or_else(|| ApiError::BadRequest(format!("未知导出类型「{}」", task.kind)))?;
    // 权限被收窄后，历史文件也下不走（列表码变了就不用给他了）
    auth.require(kind.perm)?;
    match task.status {
        2 => {}
        3 => return Err(ApiError::BadRequest(format!("导出失败，没有文件：{}", task.msg))),
        4 => return Err(ApiError::BadRequest("任务已取消，没有文件".into())),
        _ => return Err(ApiError::BadRequest("导出还没跑完，稍后刷新列表再看".into())),
    }
    // 流式回文件：不再把整份导出读进内存（几十万行就是几十 MB，并发下载会把内存顶上去）
    let file = tokio::fs::File::open(&task.file_path)
        .await
        .map_err(|_| ApiError::NotFoundMsg("文件已过期或被清理，请重新发起导出".into()))?;
    let filename = format!("{}-{}.csv", kind.file_prefix, csv::today());
    Ok(csv::stream_file(file, filename))
}

/// 取消还没跑完的任务。**标记式**：只把行置 4，不在这里等执行者停下来 ——
/// 执行者每批（5000 行）回写进度时会探到这一行不再属于它，批间收手并丢掉半份文件
/// （见 `crate::export::beat`）。所以「取消成功」= 请求受理了，不是「已经停了」。
#[apidoc::title("取消导出任务")]
#[apidoc::desc("只能取消自己的、还没跑完的任务（待处理 / 执行中）置为已取消；已经结束的 409")]
#[apidoc::url("/api/v1/exports/{id}/cancel")]
#[apidoc::method("POST")]
#[apidoc::tag("导出")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::response_status("403")]
#[apidoc::response_status("409")]
#[apidoc::returned(name = "data", ty = "object", desc = "canceled：实际改动的条数（0 = 刚好被跑完）")]
pub async fn cancel(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:export:list")?;
    let task = own(&state, &auth, &id).await?;
    // 已经有结局的行（2/3/4）不能再取消
    if !matches!(task.status, 0 | 1) {
        return Err(not_cancelable());
    }
    // 谓词带 status IN (0, 1)：上面那次读与这一写之间任务可能刚好跑完 ——
    // 那种情况命中 0 行，回同一句 409（真实结局以这一写为准，不信刚才那次读）。
    // 只改 status / finished_at / msg：进度**留在取消那一刻的值**（部分行数），
    // 文件交给执行者的看守或 `export_clean` 收，这里不碰盘
    let n = state
        .db
        .execute(
            "UPDATE export_task SET status = 4, finished_at = ?, msg = ? WHERE id = ? AND status IN (0, 1)",
            &[
                bee_orm::Value::from(now()),
                bee_orm::Value::from("已取消"),
                bee_orm::Value::from(task.id),
            ],
        )
        .await
        .map_err(ApiError::from)?;
    if n == 0 {
        return Err(not_cancelable());
    }
    Ok(ok(json!({ "canceled": n })))
}

/// 「这行已经结束了，取消不了」：409 而不是 400 —— 请求本身没毛病，是**行的状态**
/// 不允许（与手动触发在跑的 job 同一个档）。文案与前端词表 `err.export.not_cancelable`
/// 挂在一起（`ERR_TABLE` 不认它，`err` 显式带上）。
fn not_cancelable() -> ApiError {
    ApiError::Coded {
        status: StatusCode::CONFLICT,
        err: "export.not_cancelable",
        msg: "任务已经结束，不能取消".into(),
        args: Value::Null,
    }
}

/// 删任务（连文件）。误删只是丢一份能重导的文件，不做二次确认之外的限制。
#[apidoc::title("删除导出任务")]
#[apidoc::desc("删掉任务记录与导出文件（只能删自己的）")]
#[apidoc::url("/api/v1/exports/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("导出")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::response_status("200")]
#[apidoc::returned(name = "data", ty = "object", desc = "deleted：实际删除条数")]
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:export:remove")?;
    let task = own(&state, &auth, &id).await?;
    if !task.file_path.is_empty() {
        match std::fs::remove_file(&task.file_path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // 文件删不掉就不删记录：记录先没了，这个文件就再没人清了
            Err(e) => {
                return Err(ApiError::internal(format!(
                    "删除导出文件失败（{}），记录已保留，稍后可重试",
                    e
                )))
            }
        }
    }
    let deleted = state
        .db
        .execute("DELETE FROM export_task WHERE id = ?", &[bee_orm::Value::from(task.id)])
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "deleted": deleted })))
}

/// 载入任务并确认是**自己的**。别人的任务回 403（口径同数据权限越界：明确说清为什么被拒，
/// 而不是伪装成 404 —— 对外 id 是 hashid，本来也猜不出来）。
async fn own(state: &AppState, auth: &Auth, raw: &str) -> Result<ExportTask, ApiError> {
    let id = hid::dec(raw)?;
    let task = ExportTask::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if task.admin_id != auth.admin.id {
        return Err(ApiError::Forbidden("只能操作自己发起的导出".into()));
    }
    Ok(task)
}
