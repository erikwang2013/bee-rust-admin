# 管理后台服务端 · 业务 API（M4）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付管理员/角色/菜单/部门/登录记录五个模块的 CRUD 接口与数据权限，并用真库集成测试跑通全链路。

**Architecture:** 每个模块一个 `api/*.rs`，handler 内 `auth.require("权限码")` 鉴权；列表接口统一经 `datascope` 注入数据权限条件（`QuerySet::filter_raw` 参数化）；删除类接口先做引用校验再删。

**Tech Stack:** 同 M3（`docs/superpowers/plans/2026-10-05-admin-backend-core.md`）。

**设计依据:** 设计文档 §5.3/§5.4；JSON 形状以 `docs/superpowers/plans/2026-10-05-admin-web-frontend.md` 的「接口契约」为准。

**前置:** M3 完成（`admin/` 跑通登录链路）。开工前 `cargo test -p bee_admin` 全绿。

---

### Task B6: 数据权限（datascope）

**Files:**
- Create: `admin/src/datascope.rs`
- Modify: `admin/src/main.rs`（`mod datascope;`）

- [ ] **Step 1: 写 `admin/src/datascope.rs`（含纯函数单测）**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::auth::Auth;
use crate::error::ApiError;
use crate::models::Dept;
use bee_orm::{Db, Model, QuerySet};

/// 解析后的数据权限：全部 / 部门集合（并集）/ 仅本人。
#[derive(Debug, Default, Clone)]
pub struct DataScope {
    pub all: bool,
    pub dept_ids: Vec<u64>,
    pub self_only: bool,
    pub me: u64,
}

impl DataScope {
    /// 生成 `(SQL 片段, 绑定参数)`；`None` = 不加条件（全部数据）。
    /// `dept_col`：按部门过滤的列；`self_col`：按本人的列（如 `id` / `admin_id`）。
    pub fn condition(&self, dept_col: &str, self_col: &str) -> Option<(String, Vec<String>)> {
        if self.all {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        let mut params: Vec<String> = Vec::new();
        if !self.dept_ids.is_empty() {
            let ph = vec!["?"; self.dept_ids.len()].join(", ");
            parts.push(format!("{dept_col} IN ({ph})"));
            params.extend(self.dept_ids.iter().map(|d| d.to_string()));
        }
        if self.self_only {
            parts.push(format!("{self_col} = ?"));
            params.push(self.me.to_string());
        }
        if parts.is_empty() {
            return Some(("1 = 0".to_string(), Vec::new()));
        }
        Some((format!("({})", parts.join(" OR ")), params))
    }

    /// 登录记录专用：部门 → `admin_id IN (该部门下的管理员)`（参数化子查询）。
    pub fn login_log_condition(&self) -> Option<(String, Vec<String>)> {
        if self.all {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        let mut params: Vec<String> = Vec::new();
        if !self.dept_ids.is_empty() {
            let ph = vec!["?"; self.dept_ids.len()].join(", ");
            parts.push(format!(
                "admin_id IN (SELECT id FROM admin WHERE dept_id IN ({ph}))"
            ));
            params.extend(self.dept_ids.iter().map(|d| d.to_string()));
        }
        if self.self_only {
            parts.push("admin_id = ?".to_string());
            params.push(self.me.to_string());
        }
        if parts.is_empty() {
            return Some(("1 = 0".to_string(), Vec::new()));
        }
        Some((format!("({})", parts.join(" OR ")), params))
    }
}

/// 把数据权限条件注入查询（无条件时原样返回）。
pub fn apply<T: Model>(qs: QuerySet<T>, scope: &DataScope, dept_col: &str, self_col: &str) -> QuerySet<T> {
    match scope.condition(dept_col, self_col) {
        Some((sql, params)) => qs.filter_raw(sql, &params),
        None => qs,
    }
}

/// 解析当前用户的数据权限。规则：超管=全部；任一启用角色 scope=1 → 全部；
/// 否则部门集合取并集（scope 2=本部门及以下、3=本部门、5=自定义），任一角色 scope=4 → 含仅本人。
pub async fn resolve(auth: &Auth, db: &Db) -> Result<DataScope, ApiError> {
    let mut scope = DataScope { all: false, dept_ids: Vec::new(), self_only: false, me: auth.admin.id };
    if auth.is_super {
        scope.all = true;
        return Ok(scope);
    }
    let active: Vec<&crate::models::Role> = auth.roles.iter().filter(|r| r.status == 1).collect();
    if active.is_empty() {
        return Ok(scope); // 无启用角色 → 条件为 1=0（什么都看不到）
    }
    let all_depts: Vec<Dept> = Dept::query().fetch_all(db).await.map_err(ApiError::from)?;
    for role in active {
        match role.data_scope {
            1 => {
                scope.all = true;
                return Ok(scope);
            }
            2 => scope.dept_ids.extend(subtree(&all_depts, auth.admin.dept_id)),
            3 => scope.dept_ids.push(auth.admin.dept_id),
            4 => scope.self_only = true,
            5 => {
                let ids = db
                    .get_relations("role_dept", ("role_id", role.id), "dept_id")
                    .await
                    .map_err(ApiError::from)?;
                scope.dept_ids.extend(ids);
            }
            other => tracing::warn!("角色 {} 的 data_scope={other} 未知，已忽略", role.id),
        }
    }
    scope.dept_ids.sort_unstable();
    scope.dept_ids.dedup();
    Ok(scope)
}

/// 部门子树 id（含自身）。用 `visited` 防数据异常造成的环。
pub fn subtree(all: &[Dept], root: u64) -> Vec<u64> {
    let mut out: Vec<u64> = vec![root];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i];
        for d in all.iter().filter(|d| d.parent_id == parent) {
            if !out.contains(&d.id) {
                out.push(d.id);
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::now;

    fn dept(id: u64, parent_id: u64) -> Dept {
        Dept {
            id, parent_id, name: format!("d{id}"), sort: 0,
            leader: String::new(), phone: String::new(), status: 1,
            created_at: now(), updated_at: now(),
        }
    }

    #[test]
    fn subtree_is_recursive_and_cycle_safe() {
        // 1 → 2 → 3；4 无关；5 ⇄ 6 成环
        let all = vec![dept(1, 0), dept(2, 1), dept(3, 2), dept(4, 0), dept(5, 6), dept(6, 5)];
        let mut got = subtree(&all, 1);
        got.sort();
        assert_eq!(got, vec![1, 2, 3]);
        let mut cyc = subtree(&all, 5);
        cyc.sort();
        assert_eq!(cyc, vec![5, 6]);
    }

    #[test]
    fn all_scope_adds_no_condition() {
        let s = DataScope { all: true, me: 1, ..Default::default() };
        assert!(s.condition("dept_id", "id").is_none());
    }

    #[test]
    fn empty_scope_is_always_false() {
        let s = DataScope { me: 9, ..Default::default() };
        let (sql, params) = s.condition("dept_id", "id").unwrap();
        assert_eq!(sql, "1 = 0");
        assert!(params.is_empty());
    }

    #[test]
    fn dept_and_self_are_or_combined() {
        let s = DataScope { dept_ids: vec![3, 4], self_only: true, me: 9, all: false };
        let (sql, params) = s.condition("dept_id", "id").unwrap();
        assert_eq!(sql, "(dept_id IN (?, ?) OR id = ?)");
        assert_eq!(params, vec!["3", "4", "9"]);
    }
}
```

- [ ] **Step 2: 跑单测**

Run: `cargo test -p bee_admin datascope`
Expected: 4 个测试 PASS。

- [ ] **Step 3: Commit**

```bash
git add admin/src
git commit -m "feat(admin): 数据权限解析与查询注入（部门子树/仅本人/自定义，含单测）"
```

---

### Task B7: 管理员 CRUD

**Files:**
- Create: `admin/src/api/admin.rs`
- Modify: `admin/src/api/mod.rs`（`pub mod admin;` + 共享分页助手）
- Modify: `admin/src/main.rs`（挂 `/api/v1/admins` 路由）

- [ ] **Step 1: `api/mod.rs` 加共享助手**

```rust
pub mod admin;

/// (1 起始页码, 每页条数)；size 上限 100。
pub(crate) fn page_size(page: Option<u32>, size: Option<u32>) -> (usize, usize) {
    (
        page.unwrap_or(1).max(1) as usize,
        size.unwrap_or(10).clamp(1, 100) as usize,
    )
}
```

- [ ] **Step 2: 写 `admin/src/api/admin.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::page_size;
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, ok};
use crate::models::{Admin, Dept, Role};
use crate::state::AppState;
use crate::util::{hash_password, now};
use axum::Json;
use axum::extract::{Path, Query, State};
use bee_orm::{Model, OrmError};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct AdminListQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub username: Option<String>,
    pub status: Option<i8>,
    pub dept_id: Option<u64>,
}

#[derive(Deserialize)]
pub struct AdminCreateBody {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub sex: i8,
    #[serde(default)]
    pub dept_id: u64,
    #[serde(default = "status_default")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
    #[serde(default)]
    pub role_ids: Vec<u64>,
}

#[derive(Deserialize)]
pub struct AdminUpdateBody {
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub sex: i8,
    #[serde(default)]
    pub dept_id: u64,
    #[serde(default = "status_default")]
    pub status: i8,
    #[serde(default)]
    pub remark: String,
    #[serde(default)]
    pub role_ids: Vec<u64>,
}

#[derive(Deserialize)]
pub struct StatusBody {
    pub status: i8,
}

#[derive(Deserialize)]
pub struct PasswordBody {
    pub password: String,
}

#[derive(Deserialize)]
pub struct RolesBody {
    pub role_ids: Vec<u64>,
}

fn status_default() -> i8 {
    1
}

fn validate_username(u: &str) -> Result<(), ApiError> {
    let u = u.trim();
    if u.chars().count() < 3 || u.chars().count() > 64 {
        return Err(ApiError::BadRequest("用户名长度需 3-64 个字符".into()));
    }
    if !u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ApiError::BadRequest("用户名只能包含字母、数字、下划线".into()));
    }
    Ok(())
}

fn validate_password(p: &str) -> Result<(), ApiError> {
    if p.chars().count() < 6 {
        return Err(ApiError::BadRequest("密码至少 6 位".into()));
    }
    Ok(())
}

/// 列表：筛选 + 数据权限 + 分页；补 dept_name / role_names。
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<AdminListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:list")?;
    let (page, size) = page_size(q.page, q.size);

    let mut qs = Admin::query();
    if let Some(u) = q.username.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("username", u.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    if let Some(d) = q.dept_id {
        qs = qs.filter_eq("dept_id", d).map_err(ApiError::from)?;
    }

    let scope = datascope::resolve(&auth, &state.db).await?;
    qs = datascope::apply(qs, &scope, "dept_id", "id");

    let (rows, total) = qs
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;

    let dept_names: HashMap<u64, String> = Dept::query()
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|d| (d.id, d.name))
        .collect();
    let role_names: HashMap<u64, String> = Role::query()
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|r| (r.id, r.name))
        .collect();

    let mut list = Vec::with_capacity(rows.len());
    for a in rows {
        // ponytail: 每行一次关联查询（页 10 行 = 11 次查询）；行数大改批量查
        let ids = state
            .db
            .get_relations("admin_role", ("admin_id", a.id), "role_id")
            .await
            .map_err(ApiError::from)?;
        let names: Vec<String> = ids.iter().filter_map(|i| role_names.get(i).cloned()).collect();
        let mut v = serde_json::to_value(&a)
            .map_err(|e| ApiError::internal(format!("序列化失败: {e}")))?;
        v["dept_name"] = json!(dept_names.get(&a.dept_id).cloned().unwrap_or_default());
        v["role_names"] = json!(names);
        v["role_ids"] = json!(ids);
        list.push(v);
    }

    Ok(ok(json!({ "list": list, "total": total })))
}

pub async fn detail(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:list")?;
    let a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    let role_ids = state
        .db
        .get_relations("admin_role", ("admin_id", id), "role_id")
        .await
        .map_err(ApiError::from)?;
    let mut v = serde_json::to_value(&a).map_err(|e| ApiError::internal(format!("序列化失败: {e}")))?;
    v["role_ids"] = json!(role_ids);
    Ok(ok(v))
}

pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<AdminCreateBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:add")?;
    validate_username(&body.username)?;
    validate_password(&body.password)?;

    let mut a = Admin {
        id: 0,
        username: body.username.trim().to_string(),
        password: hash_password(&body.password),
        nickname: body.nickname,
        email: body.email,
        phone: body.phone,
        sex: body.sex,
        avatar: String::new(),
        dept_id: body.dept_id,
        status: body.status,
        is_super: 0,
        token_version: 0,
        last_login_at: None,
        last_login_ip: String::new(),
        remark: body.remark,
        created_at: now(),
        updated_at: now(),
    };
    match state.db.insert(&mut a).await {
        Ok(_) => {}
        Err(OrmError::DuplicateKey(_)) => return Err(ApiError::BadRequest("用户名已存在".into())),
        Err(e) => return Err(e.into()),
    }
    state
        .db
        .set_relations("admin_role", ("admin_id", a.id), "role_id", &body.role_ids)
        .await
        .map_err(ApiError::from)?;

    Ok(ok(json!({ "id": a.id })))
}

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<u64>,
    Json(body): Json<AdminUpdateBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    a.nickname = body.nickname;
    a.email = body.email;
    a.phone = body.phone;
    a.sex = body.sex;
    a.dept_id = body.dept_id;
    a.status = body.status;
    a.remark = body.remark;
    a.updated_at = now();
    state.db.update(&a).await.map_err(ApiError::from)?;
    state
        .db
        .set_relations("admin_role", ("admin_id", id), "role_id", &body.role_ids)
        .await
        .map_err(ApiError::from)?;

    Ok(ok(Value::Null))
}

pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:remove")?;
    if id == auth.admin.id {
        return Err(ApiError::BadRequest("不能删除自己".into()));
    }
    let a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if a.is_super == 1 {
        return Err(ApiError::BadRequest("不能删除超级管理员".into()));
    }
    state
        .db
        .del_relations("admin_role", "admin_id", id)
        .await
        .map_err(ApiError::from)?;
    state.db.delete::<Admin>(id).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

pub async fn set_status(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<u64>,
    Json(body): Json<StatusBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    if id == auth.admin.id {
        return Err(ApiError::BadRequest("不能修改自己的状态".into()));
    }
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if a.is_super == 1 {
        return Err(ApiError::BadRequest("不能修改超级管理员的状态".into()));
    }
    a.status = if body.status == 0 { 0 } else { 1 };
    if a.status != 1 {
        a.token_version += 1; // 禁用即踢下线
    }
    a.updated_at = now();
    state.db.update(&a).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

pub async fn reset_password(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<u64>,
    Json(body): Json<PasswordBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:resetPwd")?;
    validate_password(&body.password)?;
    let mut a = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    if a.is_super == 1 && id != auth.admin.id {
        return Err(ApiError::BadRequest("不能重置其他超级管理员的密码".into()));
    }
    a.password = hash_password(&body.password);
    a.token_version += 1; // 旧 token 立即失效
    a.updated_at = now();
    state.db.update(&a).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

pub async fn set_roles(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<u64>,
    Json(body): Json<RolesBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:admin:edit")?;
    let exists = Admin::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::from)?;
    if exists.is_none() {
        return Err(ApiError::NotFound);
    }
    state
        .db
        .set_relations("admin_role", ("admin_id", id), "role_id", &body.role_ids)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}
```

- [ ] **Step 3: `main.rs` 挂路由**

```rust
        .ns("/api/v1/admins", |ns| {
            ns.get("", api::admin::list)
                .post("", api::admin::create)
                .get("/{id}", api::admin::detail)
                .put("/{id}", api::admin::update)
                .delete("/{id}", api::admin::remove)
                .put("/{id}/status", api::admin::set_status)
                .put("/{id}/password", api::admin::reset_password)
                .put("/{id}/roles", api::admin::set_roles)
        })
```

- [ ] **Step 4: 验证**

Run: `cargo build -p bee_admin`
Expected: 编译通过。若报 axum 路径语法错误，注意本项目用 axum 0.8 的 `{id}` 写法。

- [ ] **Step 5: Commit**

```bash
git add admin/src
git commit -m "feat(admin): 管理员 CRUD（含数据权限过滤、禁用踢下线、不能删自己/超管）"
```

---

### Task B8: 角色 CRUD + 权限分配

**Files:**
- Create: `admin/src/api/role.rs`
- Modify: `admin/src/api/mod.rs`、`admin/src/main.rs`

- [ ] **Step 1: 写 `admin/src/api/role.rs`**

要点（完整代码）：`RoleListQuery{page,size,name,status}`、`RoleBody{name,code,sort,data_scope,status,remark}`、`MenuIdsBody{menu_ids}`、`DeptIdsBody{dept_ids}`。

- `list`：`system:role:list`；name 模糊 + status；`order_by("sort ASC, id ASC")`；分页；返回 `{list, total}`（直接 `serde_json::to_value` 每行，不需要补字段）。
- `detail`：`system:role:list`；不存在 → 404。
- `create`：`system:role:add`；校验 `name` 非空、`code` 3-64 且只允许 `[A-Za-z0-9_:]`、`data_scope ∈ 1..=5`；`OrmError::DuplicateKey` → 400「角色标识已存在」。
- `update`：`system:role:edit`；先读后改（`name/code/sort/data_scope/status/remark`）；code 冲突同样转 400。
- `remove`：`system:role:remove`；`AdminRole::query().filter_eq("role_id", id)?.count(&db).await? > 0` → 400「该角色已被管理员使用，不能删除」；否则 `del_relations("role_menu","role_id",id)`、`del_relations("role_dept","role_id",id)`、`delete::<Role>(id)`。
- `get_menus` / `set_menus`：`get` 用 `system:role:list`，`set` 用 `system:role:edit`；set 前校验角色存在。
- `get_depts` / `set_depts`：同上（`role_dept` 表）。

关键代码（其余按同样模式写）：

```rust
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:role:remove")?;
    let used = crate::models::AdminRole::query()
        .filter_eq("role_id", id)
        .map_err(ApiError::from)?
        .count(&state.db)
        .await
        .map_err(ApiError::from)?;
    if used > 0 {
        return Err(ApiError::BadRequest("该角色已被管理员使用，不能删除".into()));
    }
    state.db.del_relations("role_menu", "role_id", id).await.map_err(ApiError::from)?;
    state.db.del_relations("role_dept", "role_id", id).await.map_err(ApiError::from)?;
    state.db.delete::<Role>(id).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}
```

```rust
fn validate_code(code: &str) -> Result<(), ApiError> {
    if code.chars().count() < 3 || code.chars().count() > 64 {
        return Err(ApiError::BadRequest("角色标识长度需 3-64".into()));
    }
    if !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':') {
        return Err(ApiError::BadRequest("角色标识只能包含字母、数字、下划线、冒号".into()));
    }
    Ok(())
}

fn validate_scope(scope: i8) -> Result<(), ApiError> {
    if !(1..=5).contains(&scope) {
        return Err(ApiError::BadRequest("数据范围取值需在 1-5 之间".into()));
    }
    Ok(())
}
```

- [ ] **Step 2: 路由**

```rust
        .ns("/api/v1/roles", |ns| {
            ns.get("", api::role::list)
                .post("", api::role::create)
                .get("/{id}", api::role::detail)
                .put("/{id}", api::role::update)
                .delete("/{id}", api::role::remove)
                .get("/{id}/menus", api::role::get_menus)
                .put("/{id}/menus", api::role::set_menus)
                .get("/{id}/depts", api::role::get_depts)
                .put("/{id}/depts", api::role::set_depts)
        })
```

- [ ] **Step 3: 验证** Run: `cargo build -p bee_admin` → 通过

- [ ] **Step 4: Commit**

```bash
git add admin/src
git commit -m "feat(admin): 角色 CRUD 与菜单/数据权限分配（删除前校验被引用）"
```

---

### Task B9: 菜单与部门 CRUD

**Files:**
- Create: `admin/src/api/menu.rs`, `admin/src/api/dept.rs`
- Modify: `admin/src/api/mod.rs`、`admin/src/main.rs`

- [ ] **Step 1: 写 `admin/src/api/menu.rs`**

要点：`MenuBody{parent_id,name,type(rename 到 menu_type),perm,path,component,icon,sort,visible,status}`。

- `tree`：`system:menu:list`；全量 `order_by("sort ASC, id ASC")`；递归成嵌套 JSON（**含按钮节点**，管理页要看）：

```rust
fn build(all: &[Menu], parent: u64) -> Vec<Value> {
    all.iter()
        .filter(|m| m.parent_id == parent)
        .map(|m| {
            let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
            v["children"] = json!(build(all, m.id));
            v
        })
        .collect()
}
pub async fn tree(State(state): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("system:menu:list")?;
    let all = Menu::query().order_by("sort ASC, id ASC").fetch_all(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(build(&all, 0)))
}
```

- `create`：`system:menu:add`；校验 name 非空、`menu_type ∈ {"M","C","F"}`、parent_id 存在（非 0 时）。
- `update`：`system:menu:edit`；先读后改；**防环**：`parent_id` 不能是自己或自己的后代（从新 parent 向上走，撞到 id 即 400「不能把菜单挂到自己的子节点下」）。
- `remove`：`system:menu:remove`；有子节点 → 400「请先删除子菜单」；被 `role_menu` 引用 → 400「该菜单已被角色引用」；然后 `delete::<Menu>(id)`。
- 校验函数：

```rust
fn validate_type(t: &str) -> Result<(), ApiError> {
    if !matches!(t, "M" | "C" | "F") {
        return Err(ApiError::BadRequest("菜单类型只能是 M/C/F".into()));
    }
    Ok(())
}

/// 从 `parent` 向上走到根；碰到 `id` 说明会成环。
async fn would_cycle(state: &AppState, id: u64, parent: u64) -> Result<bool, ApiError> {
    let all = Menu::query().fetch_all(&state.db).await.map_err(ApiError::from)?;
    let by_id: std::collections::HashMap<u64, u64> = all.iter().map(|m| (m.id, m.parent_id)).collect();
    let mut cur = parent;
    let mut hops = 0;
    while cur != 0 && hops <= all.len() {
        if cur == id {
            return Ok(true);
        }
        cur = *by_id.get(&cur).unwrap_or(&0);
        hops += 1;
    }
    Ok(false)
}
```

- [ ] **Step 2: 写 `admin/src/api/dept.rs`**

同样的模式：`DeptBody{parent_id,name,sort,leader,phone,status}`；`tree`（`system:dept:list`，嵌套）；`create/update`（防环同 menu）；`remove`：有子部门 → 400「请先删除子部门」，部门下有管理员（`Admin::query().filter_eq("dept_id", id)?.count()`）→ 400「该部门下还有管理员」。

- [ ] **Step 3: 路由**

```rust
        .ns("/api/v1/menus", |ns| {
            ns.get("/tree", api::menu::tree)
                .post("", api::menu::create)
                .put("/{id}", api::menu::update)
                .delete("/{id}", api::menu::remove)
        })
        .ns("/api/v1/depts", |ns| {
            ns.get("/tree", api::dept::tree)
                .post("", api::dept::create)
                .put("/{id}", api::dept::update)
                .delete("/{id}", api::dept::remove)
        })
```

- [ ] **Step 4: 验证** Run: `cargo build -p bee_admin` → 通过

- [ ] **Step 5: Commit**

```bash
git add admin/src
git commit -m "feat(admin): 菜单与部门 CRUD（树形、防环、删除前引用校验）"
```

---

### Task B10: 登录记录 + 全链路集成测试 + 收尾

**Files:**
- Create: `admin/src/api/login_log.rs`
- Create: `admin/tests/common/mod.rs`、`admin/tests/api_flow_test.rs`
- Modify: `admin/src/api/mod.rs`、`admin/src/main.rs`
- Create: `README.md`（仓库根，部署与使用）

- [ ] **Step 1: 写 `admin/src/api/login_log.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::page_size;
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, ok};
use crate::models::LoginLog;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use bee_orm::Model;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct LogListQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub username: Option<String>,
    pub status: Option<i8>,
    /// "YYYY-MM-DD HH:MM:SS"
    pub start: Option<String>,
    pub end: Option<String>,
}

/// 组装筛选条件（不含数据权限），列表与清空共用。
fn filtered(q: &LogListQuery, scope: &datascope::DataScope) -> Result<bee_orm::QuerySet<LoginLog>, ApiError> {
    let mut qs = LoginLog::query();
    if let Some(u) = q.username.as_deref().filter(|s| !s.trim().is_empty()) {
        qs = qs.filter_contains("username", u.trim()).map_err(ApiError::from)?;
    }
    if let Some(s) = q.status {
        qs = qs.filter_eq("status", s).map_err(ApiError::from)?;
    }
    if let Some(s) = q.start.as_deref().filter(|s| !s.is_empty()) {
        qs = qs.filter_raw("created_at >= ?", &[s]);
    }
    if let Some(e) = q.end.as_deref().filter(|s| !s.is_empty()) {
        qs = qs.filter_raw("created_at <= ?", &[e]);
    }
    if let Some((sql, params)) = scope.login_log_condition() {
        qs = qs.filter_raw(sql, &params);
    }
    Ok(qs)
}

pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<LogListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:loginlog:list")?;
    let (page, size) = page_size(q.page, q.size);
    let scope = datascope::resolve(&auth, &state.db).await?;
    let (rows, total) = filtered(&q, &scope)?
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

/// 按当前筛选条件清空。
/// ponytail: 先取 id 再分批删（ORM 无按条件批量删）；日志量极大时改为 ORM 批量删。
pub async fn clear(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<LogListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("system:loginlog:remove")?;
    let scope = datascope::resolve(&auth, &state.db).await?;
    // 分页参数对「全量筛选」不生效：fetch_all 拿全部匹配 id
    let rows = filtered(&q, &scope)?
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::from)?;
    let mut deleted = 0u64;
    for chunk in rows.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        // id 来自数据库，非用户输入；表名写死
        let sql = format!("DELETE FROM login_log WHERE id IN ({placeholders})");
        let mut qs = sqlx::query(&sql);
        for r in chunk {
            qs = qs.bind(r.id);
        }
        qs.execute(state.db.pool()).await.map_err(|e| ApiError::from(bee_orm::OrmError::from(e)))?;
        deleted += chunk.len() as u64;
    }
    Ok(ok(json!({ "deleted": deleted })))
}
```

（`sqlx` 已是 `admin` 的 dev-dependency；本文件在 `src/` 里用到了它 → 把它从 `[dev-dependencies]` **上移到** `[dependencies]`，版本/features 不变。）

- [ ] **Step 2: 路由**

```rust
        .ns("/api/v1/login-logs", |ns| {
            ns.get("", api::login_log::list).delete("", api::login_log::clear)
        })
```

- [ ] **Step 3: 写 `admin/tests/common/mod.rs`（测试脚手架，两个测试文件共用）**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub struct Server(pub Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn dsn() -> Option<String> {
    match std::env::var("BEE_ADMIN_DB_DSN") {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("跳过：未设置 BEE_ADMIN_DB_DSN");
            None
        }
    }
}

/// 清库（服务启动时会 syncdb + seed 重建）。
pub async fn reset_db(dsn: &str) {
    let pool = sqlx::MySqlPool::connect(dsn).await.unwrap();
    for t in ["admin_role", "role_menu", "role_dept", "login_log", "menu", "role", "dept", "admin"] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {t}")).execute(&pool).await.unwrap();
    }
}

/// 用随机端口起真实进程；返回 (进程守卫, base_url)。
pub async fn start_server() -> (Server, String) {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let conf = std::fs::read_to_string("conf/app.conf.test").unwrap();
    let conf = conf.replace("127.0.0.1:0", &format!("127.0.0.1:{port}"));
    let tmp = std::env::temp_dir().join(format!("bee_admin_test_{port}.conf"));
    std::fs::write(&tmp, conf).unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_bee_admin"))
        .env("BEE_ADMIN_CONF", &tmp)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("启动 bee_admin 失败");
    let server = Server(child);

    let base = format!("http://127.0.0.1:{port}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if reqwest::get(format!("{base}/api/v1/health")).await.is_ok() {
            return (server, base);
        }
        assert!(tokio::time::Instant::now() < deadline, "服务 15 秒内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// 登录拿 token。
pub async fn login(base: &str, user: &str, pass: &str) -> Option<String> {
    let r = reqwest::Client::new()
        .post(format!("{base}/api/v1/auth/login"))
        .json(&serde_json::json!({"username": user, "password": pass}))
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v: serde_json::Value = r.json().await.ok()?;
    v["data"]["token"].as_str().map(|s| s.to_string())
}
```

（B5 已写的 `api_core_test.rs` 保留自带的脚手架；新文件用 `mod common;`。两个文件都有 `mod common` 时 cargo 会各编译一份，无冲突。）

- [ ] **Step 4: 写 `admin/tests/api_flow_test.rs`（全链路）**

覆盖（一条测试串起来，逐步断言）：

1. `reset_db` → `start_server` → 超管登录（`admin`/`admin123`）
2. `POST /depts` 建「研发部」→ 拿 id
3. `POST /roles` 建角色（`data_scope=3`）→ `PUT /roles/{id}/menus` 勾选「管理员管理」菜单（从 `GET /menus/tree` 里取 id）
4. `POST /admins` 建 `op1`（dept=研发部，role=该角色）
5. `op1` 登录 → `GET /admins`：`total == 1` 且只看到自己（数据权限=本部门）
6. `op1` `DELETE /admins/{超管id}` → **403**（无 `system:admin:remove`）
7. `op1` `POST /depts` → **403**
8. 超管 `PUT /admins/{op1}/status {"status":0}` → `op1` 的 token 再访问 → **401**（踢下线）
9. 超管 `DELETE /admins/{op1}` → 200；`GET /admins` 不再含 op1
10. `GET /login-logs` → `total >= 3`；`DELETE /login-logs?username=op1` → 只删 op1 的记录
11. `PUT /roles/{id}/depts` + `GET` 回读一致（自定义数据范围）
12. 菜单：`POST /menus` 建一个按钮 → 删掉 200；`DELETE /menus/{有子节点的目录id}` → **400**
13. 角色删除校验：删除被 `op1` 使用的角色 → 400；先删管理员再删角色 → 200

用 `common::{dsn, reset_db, start_server, login}`，每个 HTTP 调用后断言状态码与 `body["code"]`。

- [ ] **Step 5: 跑全链路测试**

```bash
BEE_ADMIN_DB_DSN='mysql://root:<密码>@127.0.0.1:3306/bee_admin_test' \
  cargo test -p bee_admin --test api_flow_test -- --nocapture
```
Expected: 全 PASS（验证时把每一步实际状态码贴进报告；不要改断言去迁就实现，实现错了就修实现）。

- [ ] **Step 6: 写仓库根 `README.md`**

内容：项目简介（bee-rust 框架 + 管理后台）、目录结构、快速开始（建库 → 复制 `admin/conf/app.conf.example` 为 `app.conf` 并填 DSN/JWT secret → `cargo run -p bee_admin` → 前端 `cd admin/web && pnpm i && pnpm dev`）、初始账号 `admin` / 配置里的 `initial_admin_password`（**登录后立即改**）、部署（systemd + nginx 8081，见设计文档 §7）、测试怎么跑（`BEE_ORM_TEST_DSN` / `BEE_ADMIN_DB_DSN`）。

- [ ] **Step 7: 全量验证**

Run: `cargo build --workspace && cargo test -p bee_admin -p bee_orm`
Expected: 全绿（未设 DSN 的集成测试跳过并打印原因）。

- [ ] **Step 8: Commit**

```bash
git add admin/ README.md
git commit -m "feat(admin): 登录记录查询/清空、全链路集成测试与 README"
```

---

## 已知取舍

- **列表关联查询 N+1**：页 10 行 = 11 次查询，管理员规模小可接受；行数大改批量。
- **清空登录记录**：先取 id 再分批删（ORM 无按条件批量删），已标 ponytail 注释。
- **角色/菜单引用校验用应用层查询**：不用外键约束，SQL 错误信息不友好且 syncdb 不管外键。
- **无操作日志**：设计文档范围外。

## Self-Review 记录

- 覆盖设计文档 §5.3 全部接口与 §5.4 数据权限五档规则；权限码与 §5.2 seed 一致（`system:admin:list/add/edit/remove/resetPwd`、`system:role:*`、`system:menu:*`、`system:dept:*`、`system:loginlog:list/remove`）。
- 与前端契约逐字段核对：`Page{list,total}`、`{id}` 返回、菜单树含 `children`、`is_super` 为 bool、时间格式 `YYYY-MM-DD HH:MM:SS`（`util::ser_dt`）。
- 路由用 axum 0.8 的 `{id}` 语法；集合路径用 `""`（bee_router 拼接 `prefix + path`，用 `"/"` 会得到带尾斜杠的路径）。
- 一致性：`datascope::{resolve, apply, DataScope}`、`api::page_size`、`ApiError::{BadRequest,Unauthorized,Forbidden,NotFound,Internal}` 在 B6-B10 中用法统一。
