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
/// `root == 0` 表示「无部门」，返回空——否则会返回整片森林（所有顶级部门及其后代）。
pub fn subtree(all: &[Dept], root: u64) -> Vec<u64> {
    if root == 0 {
        return Vec::new();
    }
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
