// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 构建期注入版本号（项 11）：release.sh 构建时传 `BRA_VERSION=$TAG`，这里原样
//! 透给代码里的 `env!("BEE_ADMIN_VERSION")`；没传（本地 cargo build / CI 测试）
//! 回落 `dev`。只用 std —— 不值得为读一个环境变量引构建依赖。
fn main() {
    // 少了这行，BRA_VERSION 变了 cargo 也不会重跑本脚本，旧版本号会一直粘在二进制里
    println!("cargo:rerun-if-env-changed=BRA_VERSION");
    let version = std::env::var("BRA_VERSION")
        .map(|v| v.trim().to_string())
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "dev".to_string());
    println!("cargo:rustc-env=BEE_ADMIN_VERSION={version}");
}
