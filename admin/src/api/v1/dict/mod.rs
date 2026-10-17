// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 字典管理（v1.5.0）：字典类型 + 字典项。
//! 除 `GET /dicts/{code}/items`（下拉数据源，登录即可）外都要权限码。
use crate::api::{PagingExt, check_len, csv, page_size};
use crate::audit::{AuditDetail, Diff};
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::hid;
use crate::models::{DictItem, DictType};
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use bee_orm::{Model, OrmError, Value as DbValue};
use serde::Deserialize;
use serde_json::{Value, json};

mod types;
mod items;

pub use self::types::*;
pub use self::items::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_accepts_only_lowercase_ident_chars() {
        assert!(valid_code("user_sex"));
        assert!(valid_code("a1"));
        assert!(valid_code(&"x".repeat(64)));
        // 长度
        assert!(!valid_code("a"), "1 位太短");
        assert!(!valid_code(&"x".repeat(65)), "65 位太长");
        // 字符集
        assert!(!valid_code("UserSex"), "大写不接受（不静默改写）");
        assert!(!valid_code("user-sex"));
        assert!(!valid_code("用户性别"));
        assert!(!valid_code("a b"));
        assert!(!valid_code(""));
    }

    #[test]
    fn empty_name_value_rejected() {
        assert!(check_name("  ").is_err());
        assert!(check_name("性别").is_ok());
        assert!(check_value(" ").is_err());
        assert!(check_value("1").is_ok());
    }
}
