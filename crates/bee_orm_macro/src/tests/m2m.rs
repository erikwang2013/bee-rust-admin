// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! §43 `m2m` expansion and compile-error tests.

use super::{expand_str, expect_error};

#[test]
fn m2m_expands_with_convention_defaults() {
    let expanded = expand_str("#[bee(m2m(Tag))] struct User { id: i64 }").expect("must expand");
    assert!(expanded.contains(
        "fn m2m () -> & 'static [bee_orm :: model :: M2mDef] { & [bee_orm :: model :: M2mDef { table : \"user_tag\" , local_column : \"user_id\" , foreign_column : \"tag_id\" , target_ident : \"Tag\" , target_table : < Tag as bee_orm :: Model > :: table_name , target_columns : < Tag as bee_orm :: Model > :: columns , }] }"
    ));
    // The target must be a `Model` for the derive to compile.
    assert!(expanded.contains(
        "const _ : fn () = || { fn assert_model < T : bee_orm :: Model > () { } assert_model :: < Tag > () ; } ;"
    ));
    // Everything else stays at its round-4 shape.
    assert!(expanded.contains("bee_orm :: QuerySet :: new (\"users\")"));
}

#[test]
fn m2m_overrides_and_repeated_targets() {
    let expanded = expand_str(
        "#[bee(m2m(Tag))] \
         #[bee(m2m(Category, table = \"user_category\", local = \"u_id\", foreign = \"c_id\"))] \
         struct User { id: i64 }",
    )
    .expect("must expand");

    assert!(expanded.contains(
        "M2mDef { table : \"user_tag\" , local_column : \"user_id\" , foreign_column : \"tag_id\" , target_ident : \"Tag\" ,"
    ));
    assert!(expanded.contains(
        "M2mDef { table : \"user_category\" , local_column : \"u_id\" , foreign_column : \"c_id\" , target_ident : \"Category\" ,"
    ));
    // One slice, one associated function, whatever the attribute count.
    assert_eq!(expanded.matches("fn m2m ()").count(), 1);
    assert_eq!(expanded.matches("assert_model :: < ").count(), 2);
}

#[test]
fn m2m_target_ident_keeps_the_attribute_spelling() {
    // §55: `target_ident` is the verbatim last-segment ident — case kept,
    // `unraw` — while §43's defaults stay lowercased. Same path, two values.
    let expanded =
        expand_str("#[bee(m2m(crate::models::Tag))] #[bee(m2m(r#type))] struct User { id: i64 }")
            .expect("must expand");
    assert!(expanded.contains("target_ident : \"Tag\" ,"));
    assert!(expanded.contains("target_ident : \"type\" ,"));
    // The same two paths through the §43 default path: lowercased, and the
    // raw `r#` prefix never reaches a literal.
    assert!(expanded.contains("table : \"user_tag\" ,"));
    assert!(expanded.contains("table : \"user_type\" ,"));
    assert!(!expanded.contains("\"r#type\""));
}

#[test]
fn m2m_self_target_and_omit_when_unused() {
    // §43: `Self` resolves to the declaring model; explicit columns make the
    // otherwise-colliding defaults legal. Inside the impl the fn paths keep
    // `Self`; the module-scope const assertion must spell the struct name.
    let expanded = expand_str(
        "#[bee(m2m(Self, local = \"manager_id\", foreign = \"report_id\"))] struct User { id: i64 }",
    )
    .expect("must expand");
    assert!(expanded.contains(
        "M2mDef { table : \"user_user\" , local_column : \"manager_id\" , foreign_column : \"report_id\" , target_ident : \"User\" , target_table : < Self as bee_orm :: Model > :: table_name , target_columns : < Self as bee_orm :: Model > :: columns , }"
    ));
    assert!(expanded.contains("assert_model :: < User > () ; } ;"));

    // Omit-when-unused: a model without `m2m` gains no tokens at all (§47).
    let expanded = expand_str("struct User { id: i64, name: String }").expect("must expand");
    assert!(!expanded.contains("M2mDef"));
    assert!(!expanded.contains("fn m2m"));
}

#[test]
fn m2m_compile_errors() {
    let duplicate = "bee_orm: duplicate m2m target: each target can appear only once";
    expect_error("#[bee(m2m(Tag))] #[bee(m2m(Tag))] struct User { id: i64 }", duplicate);
    // The same list repeats the target: still a duplicate, not a second target.
    expect_error("#[bee(m2m(Tag), m2m(Tag))] struct User { id: i64 }", duplicate);

    let collide = "bee_orm: m2m columns collide: add explicit local and foreign overrides";
    // A target ident that lowercases to the declaring model's ident.
    expect_error("#[bee(m2m(Tag))] struct Tag { id: i64 }", collide);
    // `Self` names the declaring model, so both defaults resolve to `tag_id`.
    expect_error("#[bee(m2m(Self))] struct Tag { id: i64 }", collide);
    // Explicit names that resolve equal collide the same way.
    expect_error(
        "#[bee(m2m(Tag, local = \"same\", foreign = \"same\"))] struct User { id: i64 }",
        collide,
    );

    expect_error(
        "#[bee(m2m(Tag, nope = \"x\"))] struct User { id: i64 }",
        "bee_orm: unknown m2m option (expected table, local, foreign)",
    );
    // Not pinned by §43 — chosen here, so the two cases fail loudly instead of
    // silently: keyed options with no target at all, and a repeated option key
    // (§18.3). A fully empty `m2m()` is already rejected by syn's own parser.
    expect_error(
        "#[bee(m2m(table = \"user_tag\"))] struct User { id: i64 }",
        "bee_orm: m2m requires a target type",
    );
    expect_error(
        "#[bee(m2m(Tag, table = \"a\", table = \"b\"))] struct User { id: i64 }",
        "bee_orm: duplicate #[bee(m2m)] option",
    );
    // A malformed `m2m(...)` reports its own error only — the half-read
    // attribute contributes no metadata, so nothing cascades onto it.
    expect_error(
        "#[bee(m2m(Tag, nope = \"x\"))] struct Tag { id: i64 }",
        "bee_orm: unknown m2m option (expected table, local, foreign)",
    );
}
