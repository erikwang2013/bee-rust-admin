// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! The real thing, once: `bee-rust migrate init` in a scratch crate, the user
//! fills in a sqlite model, `bee-rust migrate run` builds and executes it, and
//! the table exists afterwards.
//!
//! Env-gated by `BEE_CLI_E2E` (it spawns `cargo run`, which compiles a fresh
//! crate — minutes, not milliseconds). The inner build is online: the scratch
//! crate re-resolves from scratch, so a cold cargo cache (CI runners) lacks
//! its versions — offline only ever passed on warm local caches. Network use
//! is opt-in with the gate. Scratch dir is removed on success and kept for
//! inspection when an assertion fails.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The bee-rust workspace root: `CARGO_MANIFEST_DIR` is `crates/bee_cli`.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bee-cli-e2e-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The scratch crate: `bee_orm` (sqlite, via path to this workspace) + tokio.
fn manifest() -> String {
    format!(
        "[package]\nname = \"bee-cli-e2e\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
         [dependencies]\n\
         bee_orm = {{ path = \"{}\", features = [\"sqlite\"] }}\n\
         tokio = {{ version = \"1\", features = [\"full\"] }}\n",
        workspace().join("crates/bee_orm").display()
    )
}

/// Run the built CLI binary in `dir`.
fn cli(dir: &Path, args: &[&str]) -> std::process::ExitStatus {
    Command::new(env!("CARGO_BIN_EXE_bee-rust"))
        .args(args)
        .current_dir(dir)
        .status()
        .expect("failed to run the bee-rust binary")
}

#[test]
fn init_fill_run_creates_the_table() {
    if std::env::var_os("BEE_CLI_E2E").is_none() {
        return;
    }
    let dir = scratch();
    let db = dir.join("app.db");
    fs::write(dir.join("Cargo.toml"), manifest()).unwrap();

    // 1. scaffold.
    assert!(cli(&dir, &["migrate", "init"]).success(), "migrate init failed");
    let bin = dir.join("src/bin/bee_migrate.rs");
    assert!(bin.is_file(), "{} was not written", bin.display());

    // 2. the user fills it in: one model, and the sync line uncommented.
    let filled = fs::read_to_string(&bin).unwrap().replace(
        "// bee_orm::migrate::sync::<Post, _>(&db).await?;",
        "bee_orm::migrate::sync::<Post, _>(&db).await?;",
    ) + "\n#[derive(bee_orm::Model, Clone)]\n#[bee(table = \"posts\")]\nstruct Post {\n    \
           #[bee(pk, auto)]\n    id: i64,\n    title: String,\n}\n";
    fs::write(&bin, filled).unwrap();

    // 3. run it: `cargo run --bin bee_migrate`, online (a cold registry cannot
    // resolve the scratch crate offline — see the module docs), own target dir.
    let status = Command::new(env!("CARGO_BIN_EXE_bee-rust"))
        .args(["migrate", "run"])
        .current_dir(&dir)
        .env("DATABASE_URL", &db)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        // The outer `cargo test` jobserver must not be handed to the inner build.
        .env_remove("CARGO_MAKEFLAGS")
        .env_remove("MAKEFLAGS")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "migrate run exited with {status}; scratch kept at {}",
        dir.display()
    );

    // 4. the table exists: sqlite keeps the schema SQL in the file, with the
    // `IF NOT EXISTS` clause stripped from what the migration sent.
    let bytes = fs::read(&db).unwrap();
    let haystack = String::from_utf8_lossy(&bytes);
    assert!(
        haystack.contains(
            "CREATE TABLE posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)"
        ),
        "table not created: {}",
        db.display()
    );

    fs::remove_dir_all(&dir).unwrap();
}
