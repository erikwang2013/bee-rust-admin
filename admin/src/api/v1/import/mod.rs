// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! CSV 导入（扩展工程项 3）：管理员 / 部门 / 字典项三种。
//!
//! 契约：`GET /api/v1/imports/template?kind=…` 下模板（只有表头），
//! `POST /api/v1/imports?kind=…`（multipart 字段 `file`）导入。
//!
//! 口径（施工图「全有或全无」）：
//! 1. **先全量校验**，逐行收集报告（行号 + 原因，好行也回显 `ok:true`）；
//! 2. 只要有一行坏，**零写入**并回 `created:0` + 报告（HTTP 仍是 200 —— 请求本身被
//!    正常处理了，坏在文件内容，前端按 `created`/`rows` 渲染报告表）；
//! 3. 全对才在**单事务**里落库，中途任何一步失败整批回滚。
//!
//! 两条「不另起一套」：权限复用各模块的 `:add` 码（导入是新增数据的另一种入口），
//! 校验复用同步接口那批函数（口令策略 / 名称长度 / 数据范围闸门）——
//! 导入的参数若比同步接口宽，就是个绕过闸门的后门。
//!
//! shortcut: 只认 UTF-8（Excel 默认另存是 GBK，会被拒并提示另存为 CSV UTF-8）；
//! 等真实诉求来了再接 encoding_rs 做编码嗅探。行数上限：部门 / 字典项 5000（事务规模 +
//! 响应体），管理员 300（每行 argon2，压到一次请求能跑完）。

use crate::api::admin::{sealed, validate_profile, validate_username};
use crate::api::csv::{BOM, headers as csv_headers, row as csv_row};
use crate::api::dept::{DeptBody, validate_body};
use crate::api::dict::{check_label, check_value};
use crate::api::{check_len, dedup_ids};
use crate::auth::Auth;
use crate::datascope;
use crate::error::{ApiError, AppMultipart, ok};
use crate::models::{Admin, Dept, DictItem, DictType, Role};
use crate::state::AppState;
use crate::util::{hash_password, now};
use axum::Json;
use axum::extract::{Multipart, Query, State};
use axum::response::{IntoResponse, Response};
use bee_orm::pool::mysql::CheckedConn;
use bee_orm::{Model, Value};
use serde::Deserialize;
use serde_json::{Value as JsonVal, json};
use std::collections::HashSet;

mod pipeline;
mod rows;

pub use self::pipeline::*;
pub(crate) use self::rows::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn dept(id: i64, name: &str) -> Dept {
        Dept {
            id,
            parent_id: 0,
            name: name.into(),
            sort: 0,
            leader: String::new(),
            phone: String::new(),
            status: 1,
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn kind_parses_and_rejects_unknown() {
        assert!(Kind::parse("admin").is_ok());
        assert!(Kind::parse("dept").is_ok());
        assert!(Kind::parse("dict-item").is_ok());
        assert!(Kind::parse("dict").is_err(), "字典项的种类名是 dict-item");
        assert!(Kind::parse("").is_err());
        // 三种模板列数与解析列序一一对应（表头即契约）
        assert_eq!(Kind::Admin.headers().len(), 9);
        assert_eq!(Kind::Dept.headers().len(), 6);
        assert_eq!(Kind::DictItem.headers().len(), 6);
    }

    /// 名称定位：空 = 根；唯一命中 = 该 id；不存在 / 重名都是错误（不猜）。
    #[test]
    fn pick_dept_resolves_by_unique_name() {
        let all = vec![dept(1, "总部"), dept(2, "研发部"), dept(3, "研发部")];
        assert_eq!(pick_dept(&all, "", "部门").unwrap(), 0);
        assert_eq!(pick_dept(&all, " 总部 ", "部门").unwrap(), 1);
        let e = pick_dept(&all, "不存在", "上级部门").unwrap_err();
        assert!(reason(&e).contains("上级部门「不存在」不存在"), "{:?}", reason(&e));
        let e = pick_dept(&all, "研发部", "部门").unwrap_err();
        assert!(reason(&e).contains("不唯一"), "{:?}", reason(&e));
    }

    #[test]
    fn pick_roles_splits_and_dedups() {
        let roles: Vec<Role> = ["甲", "乙"]
            .iter()
            .enumerate()
            .map(|(i, n)| Role {
                id: i as i64 + 1,
                name: (*n).into(),
                code: format!("r{i}"),
                sort: 0,
                data_scope: 1,
                status: 1,
                remark: String::new(),
                created_at: now(),
                updated_at: now(),
            })
            .collect();
        assert_eq!(pick_roles(&roles, "").unwrap(), Vec::<i64>::new());
        // 与关联写入同一口径（`dedup_ids` 会排序去重）：重复的角色名不会插两次
        assert_eq!(pick_roles(&roles, " 乙 | 甲 |乙| ").unwrap(), vec![1, 2]);
        let e = pick_roles(&roles, "丙").unwrap_err();
        assert!(reason(&e).contains("角色「丙」不存在"), "{:?}", reason(&e));
    }

    #[test]
    fn parse_status_accepts_both_dialects() {
        for (s, want) in [("", 1), ("1", 1), ("启用", 1), (" 启用 ", 1), ("0", 0), ("禁用", 0)]
        {
            assert_eq!(parse_status(s).unwrap(), want, "{s:?}");
        }
        assert!(parse_status("2").is_err());
        assert!(parse_status("yes").is_err());
    }

    /// 管理员行要逐个 hash，行数上限单独压低；部门 / 字典项仍是 5000。
    #[test]
    fn row_caps_match_hash_cost() {
        assert_eq!(Kind::Admin.max_rows(), 300);
        assert_eq!(Kind::Dept.max_rows(), MAX_ROWS);
        assert_eq!(Kind::DictItem.max_rows(), MAX_ROWS);
    }

    #[test]
    fn parse_int_defaults_to_zero_but_rejects_junk() {
        assert_eq!(parse_int("", "排序").unwrap(), 0);
        assert_eq!(parse_int(" 12 ", "排序").unwrap(), 12);
        assert_eq!(parse_int("-3", "排序").unwrap(), -3);
        assert!(parse_int("三", "排序").is_err());
    }

    #[test]
    fn decode_strips_bom_and_rejects_non_utf8() {
        assert_eq!(decode(b"a,b").unwrap(), "a,b");
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("中文".as_bytes());
        assert_eq!(decode(&bom).unwrap(), "中文");
        // GBK 的「中」（0xD6 0xD0）不是合法 UTF-8：必须拒，且提示怎么救
        let e = decode(&[0xD6, 0xD0]).unwrap_err();
        assert!(reason(&e).contains("CSV UTF-8"), "{:?}", reason(&e));
    }
}
