// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Shared helpers for the DSN-gated integration tests.
#![allow(dead_code)] // each integration test binary uses a different subset

/// The DSN in env var `var`. `None` — with a skip notice — when it is unset or
/// blank, so a plain `cargo test` without any database stays green.
pub fn dsn(var: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => {
            eprintln!("{var} not set; skipping");
            None
        }
    }
}

/// The MySQL-family server's `(major, is_mariadb)`, from `SELECT VERSION()`.
///
/// MySQL reports `8.4.11`; MariaDB `10.11.14-MariaDB-…` — so the major alone
/// cannot distinguish "MariaDB 10" from a future MySQL 10, and vendor checks
/// need the flag.
#[cfg(feature = "mysql")]
pub async fn mysql_vendor(
    pool: &bee_orm::pool::mysql::Pool,
) -> Result<(u32, bool), bee_orm::OrmError> {
    let rows = pool.query("SELECT VERSION() AS v", &[]).await?;
    let raw = rows[0]["v"].as_str().unwrap_or_default().to_string();
    let major =
        raw.split('.').next().and_then(|major| major.parse().ok()).ok_or_else(|| {
            bee_orm::OrmError::QueryError(format!("unparseable VERSION() `{raw}`"))
        })?;
    Ok((major, raw.to_ascii_lowercase().contains("mariadb")))
}
