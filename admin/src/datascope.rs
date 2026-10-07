// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::api::{dedup_ids, ids_to_values};
use crate::auth::Auth;
use crate::error::ApiError;
use crate::models::{Admin, Dept, Menu, Role};
use bee_orm::pool::mysql::Pool;
use bee_orm::{Model, QuerySet};
use std::collections::HashSet;
use crate::relations::RelationsExt;

/// 解析后的数据权限：全部 / 部门集合（并集）/ 仅本人。
#[derive(Debug, Default, Clone)]
pub struct DataScope {
    pub all: bool,
    pub dept_ids: Vec<i64>,
    pub self_only: bool,
    pub me: i64,
}

impl DataScope {
    /// 生成 WHERE 片段；`None` = 不加条件（全部数据）。
    /// `dept_col`：按部门过滤的列；`self_col`：按本人的列（如 `id` / `admin_id`）。
    ///
    /// **值是内联的整数字面量**，不是 `?` 占位符：上游 `QuerySet` 的 `filter()`
    /// 收条件串但不收参数（`filter_eq`/`filter_in` 才有参数位），而这里拼的是
    /// `A IN (...) OR B = ...` 这种**复合**条件，构建器表达不了。
    /// 内联是安全的——`dept_ids` 与 `me` 全是 `i64`，来自 `role_dept` / 当前会话，
    /// **没有一个字节来自请求输入**；整数没有注入面。将来若有非整数条件，
    /// 必须改用带参的 `filter_eq`/`filter_in`，不能照抄这里。
    pub fn condition(&self, dept_col: &str, self_col: &str) -> Option<String> {
        if self.all {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        if !self.dept_ids.is_empty() {
            parts.push(format!("{dept_col} IN ({})", int_list(&self.dept_ids)));
        }
        if self.self_only {
            parts.push(format!("{self_col} = {}", self.me));
        }
        if parts.is_empty() {
            return Some("1 = 0".to_string());
        }
        Some(format!("({})", parts.join(" OR ")))
    }

    /// 日志类（登录记录/操作日志）专用，条件都作用在 `admin_id` 上：
    /// 部门 → `admin_id IN (该部门下的管理员)`（参数化子查询）。
    /// 同上，值同样是内联的整数字面量（见 [`DataScope::condition`] 的说明）。
    /// 这里有个子查询 `admin_id IN (SELECT id FROM admin WHERE dept_id IN (...))`：
    /// 上游的 `filter_in` 只收**值列表**，表达不了子查询，所以保留子查询形态。
    pub fn admin_id_condition(&self) -> Option<String> {
        if self.all {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        if !self.dept_ids.is_empty() {
            parts.push(format!(
                "admin_id IN (SELECT id FROM admin WHERE dept_id IN ({}))",
                int_list(&self.dept_ids)
            ));
        }
        if self.self_only {
            parts.push(format!("admin_id = {}", self.me));
        }
        if parts.is_empty() {
            return Some("1 = 0".to_string());
        }
        Some(format!("({})", parts.join(" OR ")))
    }
}

/// 把数据权限条件注入查询（无条件时原样返回）。
pub fn apply<T: Model>(qs: QuerySet<T>, scope: &DataScope, dept_col: &str, self_col: &str) -> QuerySet<T> {
    match scope.condition(dept_col, self_col) {
        Some(sql) => qs.filter(sql),
        None => qs,
    }
}

/// 日志类模块（登录记录 / 操作日志）用：条件作用在 `admin_id` 上。
pub fn apply_admin_scope<T: Model>(qs: QuerySet<T>, scope: &DataScope) -> QuerySet<T> {
    match scope.admin_id_condition() {
        Some(sql) => qs.filter(sql),
        None => qs,
    }
}

/// `[1, 2, 3]` → `"1, 2, 3"`。只接受整数（见 `condition` 的内联说明）。
fn int_list(ids: &[i64]) -> String {
    ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
}

/// 部门维度是否落在 scope 内（全部 / 部门命中）。
fn dept_covered(scope: &DataScope, dept_id: i64) -> bool {
    scope.all || scope.dept_ids.contains(&dept_id)
}

/// 单条记录的作用域闸门（B5）：目标管理员必须在操作者范围内 ——
/// 与列表同一个口径（`DataScope::condition`），否则「列表里看不见的人，详情/编辑
/// 接口照样改得到」。超管的 `scope.all` 恒为真，自然放行。
pub async fn ensure_admin_in_scope(auth: &Auth, db: &Pool, target: &Admin) -> Result<(), ApiError> {
    let scope = resolve(auth, db).await?;
    if dept_covered(&scope, target.dept_id) || (scope.self_only && target.id == scope.me) {
        return Ok(());
    }
    Err(ApiError::Forbidden("超出你的数据权限范围".into()))
}

/// 部门版（建人 / 改人时校验 `body.dept_id`）：非超管只能把人放进自己范围内。
/// 「仅本人」只认自己所在部门 —— 否则 scope=4 的角色能把人建到任意部门下。
pub async fn ensure_dept_in_scope(auth: &Auth, db: &Pool, dept_id: i64) -> Result<(), ApiError> {
    let scope = resolve(auth, db).await?;
    if dept_covered(&scope, dept_id) || (scope.self_only && dept_id == auth.admin.dept_id) {
        return Ok(());
    }
    Err(ApiError::Forbidden("超出你的数据权限范围".into()))
}

/// 不能授予某角色的原因（`None` = 能授）。规则**只有这一份**：写路径
/// （`ensure_roles_grantable`）与列表/详情的显示口径（`roles_grantable`）都走它，
/// 不会出现「列表说能授、保存时 403」。
enum Block {
    /// 已停用
    Off,
    /// data_scope 比操作者宽（1/2/5 只有超管能授）
    Wider,
    /// 角色带操作者自己没有的权限码
    ExtraPerm,
}

fn block_reason(auth: &Auth, role: &Role, perms: &HashSet<String>) -> Option<Block> {
    if auth.is_super {
        return None;
    }
    if role.status != 1 {
        return Some(Block::Off);
    }
    if !matches!(role.data_scope, 3 | 4) {
        return Some(Block::Wider);
    }
    if perms.iter().any(|p| !auth.perms.contains(p)) {
        return Some(Block::ExtraPerm);
    }
    None
}

/// 授予的角色不得超出操作者自身（B5）。超管放行；空集合放行（清空角色不是提权）；
/// 角色必须存在且启用；非超管只能授予 `data_scope ∈ {3 本部门, 4 仅本人}`，
/// 且角色的生效权限码必须是操作者权限码的子集 —— 授不出「自己都没有的能力」。
pub async fn ensure_roles_grantable(auth: &Auth, db: &Pool, role_ids: &[i64]) -> Result<(), ApiError> {
    if auth.is_super {
        return Ok(());
    }
    let ids = dedup_ids(role_ids.to_vec());
    if ids.is_empty() {
        return Ok(());
    }
    let roles = Role::query()
        .filter_in("id", &ids_to_values(&ids))
        .map_err(ApiError::from)?
        .all(db)
        .await
        .map_err(ApiError::from)?;
    // 顺带补上此前缺失的存在性校验：id 不存在一律当整体非法（停用见 block_reason）
    if roles.len() != ids.len() {
        return Err(ApiError::BadRequest("角色不存在或已停用".into()));
    }
    for r in &roles {
        match block_reason(auth, r, &role_perms(db, r.id).await?) {
            None => {}
            Some(Block::Off) => return Err(ApiError::BadRequest("角色不存在或已停用".into())),
            Some(Block::Wider) => {
                return Err(ApiError::Forbidden("不能授予数据范围更宽的角色".into()));
            }
            Some(Block::ExtraPerm) => {
                return Err(ApiError::Forbidden("不能授予包含你没有的权限的角色".into()));
            }
        }
    }
    Ok(())
}

/// 只校验**新增**的角色（B5）：保留已挂的不算授予。
/// 否则超管给某人授了个 `data_scope=1` 的角色之后，部门经理连那个人的昵称都改不了
/// —— 编辑表单会把现有的 `role_ids` 一起提交，全量校验直接把整个编辑拒了。
/// 移除角色也不校验（把人手上的权限收窄不需要谁的许可）。
pub async fn ensure_roles_added_grantable(
    auth: &Auth,
    db: &Pool,
    admin_id: i64,
    new_ids: &[i64],
) -> Result<(), ApiError> {
    let current = db
        .get_relations("admin_role", ("admin_id", admin_id), "role_id")
        .await
        .map_err(ApiError::from)?;
    let added: Vec<i64> = dedup_ids(new_ids.to_vec())
        .into_iter()
        .filter(|id| !current.contains(id))
        .collect();
    ensure_roles_grantable(auth, db, &added).await
}

/// 给角色列表/详情标 `grantable`（B5 显示口径）：只做提示，写路径仍走硬校验。
/// 前端拿不到角色的权限码集合，判不了这件事，所以由后端给权威结论。
/// ponytail: 每行一次 `role_perms`（2 条查询）；角色是配置量级（几十个），不分批。
pub async fn roles_grantable(auth: &Auth, db: &Pool, roles: &[Role]) -> Result<Vec<bool>, ApiError> {
    if auth.is_super {
        return Ok(vec![true; roles.len()]);
    }
    let mut out = Vec::with_capacity(roles.len());
    for r in roles {
        out.push(block_reason(auth, r, &role_perms(db, r.id).await?).is_none());
    }
    Ok(out)
}

/// 角色当前生效的权限码（口径同 `Auth` 装载：role_menu → 启用菜单的非空 perm）。
async fn role_perms(db: &Pool, role_id: i64) -> Result<HashSet<String>, ApiError> {
    let menu_ids = db
        .get_relations("role_menu", ("role_id", role_id), "menu_id")
        .await
        .map_err(ApiError::from)?;
    if menu_ids.is_empty() {
        return Ok(HashSet::new());
    }
    Ok(Menu::query()
        .filter_in("id", &ids_to_values(&menu_ids))
        .map_err(ApiError::from)?
        .filter_eq("status", 1)
        .map_err(ApiError::from)?
        .all(db)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .filter(|m| !m.perm.is_empty())
        .map(|m| m.perm)
        .collect())
}

/// 解析当前用户的数据权限。规则：超管=全部；任一启用角色 scope=1 → 全部；
/// 否则部门集合取并集（scope 2=本部门及以下、3=本部门、5=自定义），任一角色 scope=4 → 含仅本人。
pub async fn resolve(auth: &Auth, db: &Pool) -> Result<DataScope, ApiError> {
    let mut scope = DataScope { all: false, dept_ids: Vec::new(), self_only: false, me: auth.admin.id };
    if auth.is_super {
        scope.all = true;
        return Ok(scope);
    }
    let active: Vec<&crate::models::Role> = auth.roles.iter().filter(|r| r.status == 1).collect();
    if active.is_empty() {
        return Ok(scope); // 无启用角色 → 条件为 1=0（什么都看不到）
    }
    let all_depts: Vec<Dept> = Dept::query().all(db).await.map_err(ApiError::from)?;
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
/// `root == 0` 表示「无部门」，返回空——否则会返回整片森林（所有顶级部门及其后代）。
pub fn subtree(all: &[Dept], root: i64) -> Vec<i64> {
    if root == 0 {
        return Vec::new();
    }
    let mut out: Vec<i64> = vec![root];
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

    fn dept(id: i64, parent_id: i64) -> Dept {
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
    fn subtree_of_no_dept_is_empty() {
        // dept_id=0 是「无部门」（模型默认值、前端选项「（无）」），不是根：
        // 返回整片森林会让「本部门及以下」看到全公司
        let all = vec![dept(1, 0), dept(2, 1)];
        assert!(subtree(&all, 0).is_empty());
    }

    #[test]
    fn all_scope_adds_no_condition() {
        let s = DataScope { all: true, me: 1, ..Default::default() };
        assert!(s.condition("dept_id", "id").is_none());
    }

    #[test]
    fn empty_scope_is_always_false() {
        let s = DataScope { me: 9, ..Default::default() };
        // 值是**内联的整数字面量**（不是 ? 占位符）——见 condition() 的说明
        assert_eq!(s.condition("dept_id", "id").unwrap(), "1 = 0");
    }

    #[test]
    fn dept_and_self_are_or_combined() {
        let s = DataScope { dept_ids: vec![3, 4], self_only: true, me: 9, all: false };
        assert_eq!(
            s.condition("dept_id", "id").unwrap(),
            "(dept_id IN (3, 4) OR id = 9)"
        );
    }
}
