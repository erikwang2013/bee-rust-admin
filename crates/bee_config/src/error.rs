// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("file not found: {0}")]
    NotFound(String),

    #[error("missing required key: {0}")]
    MissingKey(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("deserialization error: {0}")]
    Deserialize(String),

    #[error("config type {0} already loaded from {1}; cannot load from {2}")]
    PathConflict(String, PathBuf, PathBuf),
}
