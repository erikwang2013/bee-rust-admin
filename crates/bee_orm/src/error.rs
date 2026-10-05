// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use sqlx::Error as SqlxError;

#[derive(Debug, thiserror::Error)]
pub enum OrmError {
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("query error: {0}")]
    QueryError(String),
    #[error("invalid field name: {0}")]
    InvalidField(String),
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Sqlx(#[from] SqlxError),
    #[error("row mapping error: {0}")]
    Row(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    #[error("duplicate key: {0}")]
    DuplicateKey(String),
}

/// 把 sqlx 错误归一化：唯一键冲突（MySQL 1062，SQLSTATE 23000）单独成类，
/// 上层可转成「用户名已存在」这类友好提示。
pub(crate) fn normalize(e: SqlxError) -> OrmError {
    if let SqlxError::Database(db) = &e {
        if db.code().as_deref() == Some("23000") {
            return OrmError::DuplicateKey(db.message().to_string());
        }
    }
    OrmError::Sqlx(e)
}

/// 标识符（表名/列名）只允许 `[A-Za-z_][A-Za-z0-9_]*`；其余一律拒绝，
/// 防止拼进 SQL 造成注入。
pub(crate) fn validate_ident(name: &str) -> Result<(), OrmError> {
    let mut chars = name.chars();
    let ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok { Ok(()) } else { Err(OrmError::InvalidField(name.to_string())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_ident_accepts_plain_names() {
        assert!(validate_ident("admin_role").is_ok());
        assert!(validate_ident("_x1").is_ok());
    }

    #[test]
    fn validate_ident_rejects_injection() {
        assert!(validate_ident("admin; DROP TABLE x").is_err());
        assert!(validate_ident("1abc").is_err());
        assert!(validate_ident("").is_err());
    }
}
