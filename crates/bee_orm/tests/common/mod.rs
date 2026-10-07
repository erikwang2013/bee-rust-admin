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
