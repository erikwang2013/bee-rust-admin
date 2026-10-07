// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Compile-failure UI tests ([trybuild]): each `tests/ui/*.rs` case must fail to
//! compile with exactly the `.stderr` file next to it.

#[test]
fn compile_failures() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
    // The pass case links the generated code against the real `bee_orm`:
    // `#[bee(hooks(...))]` forwarders must resolve without recursion.
    cases.pass("tests/ui/pass/*.rs");
}
