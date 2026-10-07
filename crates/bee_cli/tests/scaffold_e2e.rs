// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! The real thing, once: `bee-rust new` scaffolds a project, `bee-rust
//! generate controller` adds a controller, and the project compiles as-is.
//! Guards against generated templates that only look right — the 1.2.2
//! controller template was not (missing `#[async_trait]`, private
//! `RouterError` path) and neither was the scaffold manifest (it named a
//! package that does not resolve: `bee-rust`, published as `bee_rust`).
//!
//! Env-gated by `BEE_CLI_E2E` (it spawns `cargo check`, which compiles the
//! framework from scratch — minutes, not milliseconds). The scaffold's
//! `bee_rust = "1"` is rewritten to a path dependency on this workspace so the
//! inner build tests the tree under development without a registry fetch of
//! the framework; the remaining crate versions resolve from the cargo cache.
//! Scratch dir is removed on success and kept for inspection on failure.

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
        "bee-cli-scaffold-e2e-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
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
fn scaffolded_project_with_a_controller_compiles() {
    if std::env::var_os("BEE_CLI_E2E").is_none() {
        return;
    }
    let dir = scratch();

    // 1. scaffold + generate, exactly as a user would. `generate` resolves
    // `controllers/` against the cwd and the scaffold's module tree is
    // `src/controllers/`, so it runs from `src/`.
    assert!(cli(&dir, &["new", "app"]).success(), "new failed");
    let app = dir.join("app");
    let src = app.join("src");
    assert!(cli(&src, &["generate", "controller", "user"]).success(), "generate failed");
    assert!(src.join("controllers/user.rs").is_file());

    // 2. wire the generated controller in: the mod list is the user's step.
    fs::write(app.join("src/controllers/mod.rs"), "pub mod user;\n").unwrap();

    // 3. the scaffold compiles against this workspace (path dep: the live tree).
    let manifest = app.join("Cargo.toml");
    let dep =
        format!("bee_rust = {{ path = \"{}\" }}", workspace().join("crates/bee_rust").display());
    let rewritten = fs::read_to_string(&manifest).unwrap().replace("bee_rust = \"1\"", &dep);
    fs::write(&manifest, rewritten).unwrap();

    let status = Command::new("cargo")
        .arg("check")
        .current_dir(&app)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        // The outer `cargo test` jobserver must not be handed to the inner build.
        .env_remove("CARGO_MAKEFLAGS")
        .env_remove("MAKEFLAGS")
        .status()
        .expect("failed to run cargo check");
    assert!(status.success(), "scaffold did not compile; scratch kept at {}", dir.display());

    fs::remove_dir_all(&dir).unwrap();
}
