// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Expansion and compile-error tests for the derive macro.

use super::expand;
use syn::DeriveInput;

mod m2m;

/// Expansion as whitespace-normalised source, or the error message.
fn expand_str(source: &str) -> Result<String, String> {
    let input: DeriveInput = syn::parse_str(source).expect("test input must parse");
    expand(input)
        .map(|tokens| tokens.to_string().split_whitespace().collect::<Vec<_>>().join(" "))
        .map_err(|error| error.to_string())
}

fn expect_error(source: &str, message: &str) {
    match expand_str(source) {
        Ok(expanded) => panic!("`{source}` should not expand: {expanded}"),
        Err(actual) => assert_eq!(actual, message, "for `{source}`"),
    }
}

#[test]
fn compile_errors_match_the_frozen_table() {
    expect_error("enum Foo { A }", "bee_orm: Model can only be derived for a struct");
    expect_error("struct Foo(u8);", "bee_orm: Model requires a struct with named fields");
    expect_error("struct Foo;", "bee_orm: Model requires a struct with named fields");
    expect_error("struct Foo<T> { id: T }", "bee_orm: Model does not support generic structs");
    expect_error(
        "struct Foo<'a> { id: &'a str }",
        "bee_orm: Model does not support generic structs",
    );
    expect_error(
        "#[bee(nope)] struct Foo { id: i64 }",
        "bee_orm: unknown bee attribute for a struct (expected table, hooks, m2m, crate)",
    );
    expect_error(
        "#[bee(table = \"a\", table = \"b\")] struct Foo { id: i64 }",
        "bee_orm: duplicate #[bee(table)] attribute",
    );
    expect_error(
        "#[bee(table = \"a\")] #[bee(table = \"b\")] struct Foo { id: i64 }",
        "bee_orm: duplicate #[bee(table)] attribute",
    );
    expect_error(
        "struct Foo { #[bee(nope)] id: i64 }",
        "bee_orm: unknown bee attribute for a field (expected column, pk, auto, ignore, auto_now_add, auto_now, soft_delete, sql_type, fk)",
    );
    expect_error(
        "struct Foo { #[bee(pk)] a: i64, #[bee(pk)] b: i64 }",
        "bee_orm: multiple #[bee(pk)] fields (`a`, `b`)",
    );
    expect_error(
        "struct Foo { #[bee(pk, ignore)] id: i64 }",
        "bee_orm: #[bee(pk)] and #[bee(ignore)] on the same field",
    );
    expect_error(
        "struct Foo { #[bee(pk)] a: i64, #[bee(auto)] b: i64 }",
        "bee_orm: #[bee(auto)] is only valid on the primary key field",
    );
    // §18.2: with no pk at all, only the "no primary key" error is reported.
    expect_error(
        "struct Foo { #[bee(auto)] name: String }",
        "bee_orm: no primary key: mark a field #[bee(pk)] or name it `id`",
    );
    expect_error(
        "struct Foo { name: String }",
        "bee_orm: no primary key: mark a field #[bee(pk)] or name it `id`",
    );
    expect_error(
        "struct Foo { #[bee(pk)] a: i64, #[bee(column = \"a\")] b: i64 }",
        "bee_orm: duplicate column name `a`",
    );
    expect_error(
        "struct Foo { #[bee(column = \"1bad\")] id: i64 }",
        "bee_orm: invalid column name `1bad` (expected [A-Za-z_][A-Za-z0-9_]*)",
    );
}

#[test]
fn round3_compile_errors() {
    expect_error(
        "#[bee(hooks(nope))] struct Foo { id: i64 }",
        "bee_orm: unknown hook `nope` (expected before_insert, after_insert, before_update, after_update, before_delete, after_delete)",
    );
    expect_error(
        "struct Foo { #[bee(auto_now_add, auto_now)] id: i64 }",
        "bee_orm: #[bee(auto_now_add)] and #[bee(auto_now)] on the same field",
    );
    expect_error(
        "struct Foo { #[bee(pk)] k: i64, #[bee(auto_now_add, ignore)] a: i64 }",
        "bee_orm: #[bee(auto_now_add)] is not valid on an ignored field",
    );
    expect_error(
        "struct Foo { #[bee(pk)] k: i64, #[bee(auto_now, ignore)] a: i64 }",
        "bee_orm: #[bee(auto_now)] is not valid on an ignored field",
    );
    expect_error(
        "struct Foo { #[bee(pk, auto, auto_now_add)] id: i64 }",
        "bee_orm: #[bee(auto_now_add)] is not valid on a database-assigned (auto) field",
    );
    expect_error(
        "struct Foo { #[bee(pk, auto, auto_now)] id: i64 }",
        "bee_orm: #[bee(auto_now)] is not valid on a database-assigned (auto) field",
    );
    expect_error(
        "struct Foo { id: i64, #[bee(soft_delete)] a: bool, #[bee(soft_delete)] b: bool }",
        "bee_orm: multiple #[bee(soft_delete)] fields",
    );
    expect_error(
        "struct Foo { id: i64, #[bee(soft_delete, ignore)] a: bool }",
        "bee_orm: #[bee(soft_delete)] is not valid on an ignored field",
    );
    expect_error(
        "struct Foo { #[bee(pk, soft_delete)] id: i64 }",
        "bee_orm: #[bee(soft_delete)] is not valid on the primary key field",
    );
    // The implicit `id` pk is caught the same way.
    expect_error(
        "struct Foo { #[bee(soft_delete)] id: i64 }",
        "bee_orm: #[bee(soft_delete)] is not valid on the primary key field",
    );
}

#[test]
fn round4_compile_errors() {
    expect_error(
        "struct U64 { #[bee(pk)] id: i64, count: u64 }",
        "bee_orm: no SQL type mapping for `u64`: add #[bee(sql_type = \"...\")] to override",
    );
    // Unmapped spellings report once: the auto-integer check does not cascade.
    expect_error(
        "struct U64 { #[bee(pk, auto)] id: u64 }",
        "bee_orm: no SQL type mapping for `u64`: add #[bee(sql_type = \"...\")] to override",
    );
    expect_error(
        "struct AutoStr { #[bee(pk, auto)] id: String }",
        "bee_orm: #[bee(auto)] requires an integer primary key: add #[bee(sql_type = \"...\")] to override",
    );
    expect_error(
        "struct FkIgnore { id: i64, #[bee(fk = User, ignore)] u: i64 }",
        "bee_orm: #[bee(fk)] is not valid on an ignored field",
    );
    expect_error(
        "struct FkPk { #[bee(pk, fk = User)] id: i64 }",
        "bee_orm: #[bee(fk)] is not valid on the primary key field",
    );
}

#[test]
fn expands_the_frozen_example() {
    let expanded = expand_str(
        "#[bee(table = \"user_accounts\")] struct User { \
             #[bee(pk, auto)] id: i64, \
             #[bee(column = \"user_name\")] name: String, \
             age: Option<i32>, \
             #[bee(ignore)] avatar_cache: Vec<u8>, \
         }",
    )
    .expect("the spec example must expand");

    assert!(expanded.contains("fn table_name () -> & 'static str { \"user_accounts\" }"));
    assert!(expanded.contains("fn pk_column () -> & 'static str { \"id\" }"));
    assert!(expanded.contains("id : bee_orm :: decode (row , \"id\") ?"));
    assert!(expanded.contains("name : bee_orm :: decode (row , \"user_name\") ?"));
    assert!(expanded.contains("avatar_cache : :: core :: default :: Default :: default ()"));
    assert!(
        expanded.contains("(\"user_name\" , bee_orm :: Value :: from (self . name . clone ()))")
    );
    assert!(expanded.contains("(\"age\" , bee_orm :: Value :: from (self . age . clone ()))"));
    assert!(expanded.contains("bee_orm :: Value :: from (self . id . clone ())"));
    // `insert_values` skips nothing here but the auto pk; `update_values`
    // skips the pk and the ignored field — neither may re-list them.
    assert!(!expanded.contains("(\"id\""));
    assert!(!expanded.contains("avatar_cache\""));
    assert!(!expanded.contains("self . avatar_cache"));
    // The inherent helpers are kept.
    assert!(expanded.contains("impl User {"));
    assert!(expanded.contains("pub fn query () -> bee_orm :: QuerySet < Self >"));
    assert!(expanded.contains("bee_orm :: QuerySet :: new (\"user_accounts\")"));
    // §37.4 baseline: the ignored field is not a `ColumnDef` either.
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : true , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"user_name\" , sql : bee_orm :: model :: SqlType :: Text , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"age\" , sql : bee_orm :: model :: SqlType :: Int , nullable : true , primary_key : false , auto_increment : false , default : None , references : None , }] }"));
}

#[test]
fn defaults_apply_without_attributes() {
    let expanded = expand_str("struct User { id: i64, name: String }").expect("must expand");
    assert!(expanded.contains("fn pk_column () -> & 'static str { \"id\" }"));
    assert!(expanded.contains("QuerySet :: new (\"users\")"));
    // No `auto` means the pk is written on insert; no pk column in update.
    assert!(expanded.contains("(\"id\" , bee_orm :: Value :: from (self . id . clone ()))"));
    assert!(expanded.contains("(\"name\" , bee_orm :: Value :: from (self . name . clone ()))"));
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"name\" , sql : bee_orm :: model :: SqlType :: Text , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , }] }"));
}

#[test]
fn explicit_pk_and_auto_pk_resolution() {
    // `auto` on the implicit `id` pk is valid (it is the primary key field).
    let expanded = expand_str("struct Foo { #[bee(auto)] id: i64 }").expect("must expand");
    assert!(expanded.contains("fn pk_column () -> & 'static str { \"id\" }"));
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : true , default : None , references : None , }] }"));

    // An explicit pk wins over a field named `id`; other fields stay mapped.
    let expanded =
        expand_str("struct Foo { #[bee(pk)] code: String, id: i64 }").expect("must expand");
    assert!(expanded.contains("fn pk_column () -> & 'static str { \"code\" }"));
    assert!(expanded.contains("bee_orm :: Value :: from (self . code . clone ())"));
    assert!(expanded.contains("(\"id\" , bee_orm :: Value :: from (self . id . clone ()))"));
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"code\" , sql : bee_orm :: model :: SqlType :: Text , nullable : false , primary_key : true , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , }] }"));

    let expanded =
        expand_str("struct Foo { #[bee(pk)] #[bee(auto)] code: i64 }").expect("must expand");
    assert!(expanded.contains("fn pk_column () -> & 'static str { \"code\" }"));
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"code\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : true , default : None , references : None , }] }"));
}

#[test]
fn raw_identifiers_lose_the_prefix_in_column_names() {
    let expanded = expand_str("struct Foo { id: i64, r#type: String }").expect("must expand");
    assert!(expanded.contains("(\"type\" , bee_orm :: Value :: from (self . r#type . clone ()))"));
    assert!(expanded.contains("r#type : bee_orm :: decode (row , \"type\") ?"));
    assert!(!expanded.contains("\"r#type\""));
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"type\" , sql : bee_orm :: model :: SqlType :: Text , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , }] }"));

    // `r#id` is still the implicit primary key.
    let expanded = expand_str("struct Bar { r#id: i64, name: String }").expect("must expand");
    assert!(expanded.contains("fn pk_column () -> & 'static str { \"id\" }"));
    assert!(expanded.contains("bee_orm :: Value :: from (self . r#id . clone ())"));
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"name\" , sql : bee_orm :: model :: SqlType :: Text , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , }] }"));

    // A raw struct name defaults the table name without the prefix.
    let expanded = expand_str("struct r#User { id: i64 }").expect("must expand");
    assert!(expanded.contains("fn table_name () -> & 'static str { \"users\" }"));
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : false , default : None , references : None , }] }"));
}

#[test]
fn round3_attributes_expand() {
    let expanded = expand_str(
        "struct Post { #[bee(pk)] id: i64, \
             #[bee(auto_now_add)] created_at: i64, \
             #[bee(auto_now, column = \"updated\")] updated_at: i64, \
             #[bee(soft_delete)] deleted: bool, \
             body: String }",
    )
    .expect("must expand");

    // The declarative column lists appear only when the attributes are used.
    assert!(expanded.contains(
        "fn auto_now_add_columns () -> & 'static [& 'static str] { & [\"created_at\"] }"
    ));
    assert!(
        expanded
            .contains("fn auto_now_columns () -> & 'static [& 'static str] { & [\"updated\"] }")
    );
    assert!(
        expanded.contains(
            "fn soft_delete_column () -> Option < & 'static str > { Some (\"deleted\") }"
        )
    );

    // Timestamps leave both value lists; the soft-delete flag stays in both.
    assert!(expanded.contains("(\"body\" , bee_orm :: Value :: from (self . body . clone ()))"));
    assert!(
        expanded.contains("(\"deleted\" , bee_orm :: Value :: from (self . deleted . clone ()))")
    );
    assert!(!expanded.contains("(\"created_at\""));
    assert!(!expanded.contains("(\"updated\""));
    // `from_row` still decodes the timestamp columns.
    assert!(expanded.contains("created_at : bee_orm :: decode (row , \"created_at\") ?"));
    assert!(expanded.contains("updated_at : bee_orm :: decode (row , \"updated\") ?"));
    // No hooks: the impl stays unannotated.
    assert!(!expanded.contains("__private"));
    // §34: timestamps force BigInt / NOT NULL / DEFAULT 0, the soft-delete flag
    // NOT NULL / DEFAULT FALSE; the renamed timestamp column keeps its override.
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"created_at\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : false , auto_increment : false , default : Some (bee_orm :: model :: DefaultValue :: Int (0)) , references : None , } , bee_orm :: model :: ColumnDef { name : \"updated\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : false , auto_increment : false , default : Some (bee_orm :: model :: DefaultValue :: Int (0)) , references : None , } , bee_orm :: model :: ColumnDef { name : \"deleted\" , sql : bee_orm :: model :: SqlType :: Bool , nullable : false , primary_key : false , auto_increment : false , default : Some (bee_orm :: model :: DefaultValue :: Bool (false)) , references : None , } , bee_orm :: model :: ColumnDef { name : \"body\" , sql : bee_orm :: model :: SqlType :: Text , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , }] }"));
}

#[test]
fn timestamps_on_the_primary_key_are_allowed() {
    // §28.2: a timestamp primary key is legitimate; only `auto` + timestamp errors.
    let expanded =
        expand_str("struct Foo { #[bee(pk, auto_now_add)] id: i64 }").expect("must expand");
    assert!(
        expanded.contains("fn auto_now_add_columns () -> & 'static [& 'static str] { & [\"id\"] }")
    );
    assert!(!expanded.contains("(\"id\""));
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : false , default : Some (bee_orm :: model :: DefaultValue :: Int (0)) , references : None , }] }"));
}

#[test]
fn hooks_expand_and_deduplicate() {
    let expanded = expand_str(
        "#[bee(hooks(before_insert, after_delete, before_insert))] struct Order { id: i64 }",
    )
    .expect("must expand");

    assert!(
        expanded
            .contains("# [bee_orm :: __private :: async_trait] impl bee_orm :: Model for Order")
    );
    assert!(expanded.contains(
        "async fn before_insert (& self) -> bee_orm :: Result < () > { Self :: before_insert (self) . await }"
    ));
    assert!(expanded.contains(
        "async fn after_delete (& self) -> bee_orm :: Result < () > { Self :: after_delete (self) . await }"
    ));
    // The duplicate is deduplicated, and unlisted hooks are not forwarded.
    assert_eq!(expanded.matches("async fn before_insert").count(), 1);
    assert!(!expanded.contains("after_insert"));
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : false , default : None , references : None , }] }"));
}

#[test]
fn spelling_table_maps_every_supported_type() {
    let expanded = expand_str(
        "struct Row4 { #[bee(pk, auto)] id: i64, note: Option<String>, data: Vec<u8>, \
             score: f32, ratio: f64, flag: bool, small: i16, amount: i32, wide: u32, \
             title: String, parent: i64, #[bee(column = \"custom\")] extra: f64, \
             born: NaiveDate, stamp: NaiveDateTime, money: Decimal, at: DateTime<Utc>, \
             opt_at: Option<DateTime<Utc>>, \
             #[bee(ignore)] cache: String }",
    )
    .expect("must expand");

    // §34 spelling table: bool → Bool, i8/i16/i32/u8/u16/u32 → Int, i64 →
    // BigInt, f32 → Real, f64 → Double, String → Text, Vec<u8> → Blob;
    // `Option` only clears the NOT NULL, and `ignore` is not a column at all.
    assert!(expanded.contains("fn columns () -> & 'static [bee_orm :: model :: ColumnDef] { & [bee_orm :: model :: ColumnDef { name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : true , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"note\" , sql : bee_orm :: model :: SqlType :: Text , nullable : true , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"data\" , sql : bee_orm :: model :: SqlType :: Blob , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"score\" , sql : bee_orm :: model :: SqlType :: Real , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"ratio\" , sql : bee_orm :: model :: SqlType :: Double , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"flag\" , sql : bee_orm :: model :: SqlType :: Bool , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"small\" , sql : bee_orm :: model :: SqlType :: Int , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"amount\" , sql : bee_orm :: model :: SqlType :: Int , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"wide\" , sql : bee_orm :: model :: SqlType :: Int , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"title\" , sql : bee_orm :: model :: SqlType :: Text , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"parent\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , bee_orm :: model :: ColumnDef { name : \"custom\" , sql : bee_orm :: model :: SqlType :: Double , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , }"));
    // §56 additions: the chrono / rust_decimal spellings, declared after the
    // §34 fields so the frozen baseline above stays a literal prefix.
    assert!(expanded.contains(
        "bee_orm :: model :: ColumnDef { name : \"born\" , sql : bee_orm :: model :: SqlType :: Date , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , \
         bee_orm :: model :: ColumnDef { name : \"stamp\" , sql : bee_orm :: model :: SqlType :: DateTime , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , \
         bee_orm :: model :: ColumnDef { name : \"money\" , sql : bee_orm :: model :: SqlType :: Decimal , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , \
         bee_orm :: model :: ColumnDef { name : \"at\" , sql : bee_orm :: model :: SqlType :: DateTimeTz , nullable : false , primary_key : false , auto_increment : false , default : None , references : None , } , \
         bee_orm :: model :: ColumnDef { name : \"opt_at\" , sql : bee_orm :: model :: SqlType :: DateTimeTz , nullable : true , primary_key : false , auto_increment : false , default : None , references : None , }] }"
    ));
    assert!(!expanded.contains("cache\""));
}

#[test]
fn sql_type_overrides_the_spelling() {
    let expanded = expand_str(
        "struct Raw { #[bee(pk)] id: i64, \
             #[bee(sql_type = \"VARCHAR(64)\")] name: String, \
             #[bee(sql_type = \"TIMESTAMP\", fk = User)] when: String }",
    )
    .expect("must expand");

    assert!(expanded.contains("sql : bee_orm :: model :: SqlType :: Raw (\"VARCHAR(64)\")"));
    assert!(expanded.contains("sql : bee_orm :: model :: SqlType :: Raw (\"TIMESTAMP\")"));
    // The override wins the type string only: the mapped spelling is gone.
    assert!(!expanded.contains("sql : bee_orm :: model :: SqlType :: Text"));
    // The `fk` in the same attribute still applies.
    assert!(expanded.contains(
        "references : Some (bee_orm :: model :: Reference { table : < User as bee_orm :: Model > :: table_name , pk_column : < User as bee_orm :: Model > :: pk_column , })"
    ));
    assert!(expanded.contains(
        "const _ : fn () = || { fn assert_model < T : bee_orm :: Model > () { } assert_model :: < User > () ; } ;"
    ));
}

#[test]
fn fk_emits_a_reference_and_asserts_the_target_model() {
    let expanded = expand_str("struct Fk { #[bee(pk)] id: i64, #[bee(fk = User)] user_id: i64 }")
        .expect("must expand");
    assert!(expanded.contains(
        "references : Some (bee_orm :: model :: Reference { table : < User as bee_orm :: Model > :: table_name , pk_column : < User as bee_orm :: Model > :: pk_column , })"
    ));
    // Const assertion: the target must be a `Model` for the derive to compile.
    assert!(expanded.contains(
        "const _ : fn () = || { fn assert_model < T : bee_orm :: Model > () { } assert_model :: < User > () ; } ;"
    ));
    // Non-fk columns keep `references : None`.
    assert!(expanded.contains(
        "name : \"id\" , sql : bee_orm :: model :: SqlType :: BigInt , nullable : false , primary_key : true , auto_increment : false , default : None , references : None , }"
    ));
}

#[test]
fn serde_json_spellings_map_to_json() {
    let expanded = expand_str(
        "struct User { id: i64, meta: serde_json::Value, \
             extra: Option<serde_json::value::Value> }",
    )
    .expect("must expand");

    assert!(expanded.contains(
        "name : \"meta\" , sql : bee_orm :: model :: SqlType :: Json , nullable : false , primary_key : false"
    ));
    // `Option` only clears NOT NULL, and the deeper path maps the same way.
    assert!(expanded.contains(
        "name : \"extra\" , sql : bee_orm :: model :: SqlType :: Json , nullable : true , primary_key : false"
    ));
}

#[test]
fn round5_json_compile_errors() {
    // §45: `Value` is ambiguous with `bee_orm::Value` — unmapped, with a hint.
    expect_error(
        "struct User { id: i64, meta: Value }",
        "bee_orm: no SQL type mapping for `Value`: use `serde_json::Value` or add #[bee(sql_type = \"...\")] to override",
    );
    expect_error(
        "struct User { id: i64, meta: Option<Value> }",
        "bee_orm: no SQL type mapping for `Value`: use `serde_json::Value` or add #[bee(sql_type = \"...\")] to override",
    );
    expect_error(
        "struct User { id: i64, meta: bee_orm::Value }",
        "bee_orm: no SQL type mapping for `bee_orm::Value`: use `serde_json::Value` or add #[bee(sql_type = \"...\")] to override",
    );
}

#[test]
fn round7_date_time_spellings_need_utc() {
    // §56: `DateTime` maps only in UTC — the bare spelling and every other
    // zone fall to the explicit-override error, never a silently wrong column.
    expect_error(
        "struct Row { #[bee(pk)] id: i64, at: DateTime }",
        "bee_orm: no SQL type mapping for `DateTime`: add #[bee(sql_type = \"...\")] to override",
    );
    expect_error(
        "struct Row { #[bee(pk)] id: i64, at: DateTime<Local> }",
        "bee_orm: no SQL type mapping for `DateTime<Local>`: add #[bee(sql_type = \"...\")] to override",
    );
    // A `NaiveTime` (out of §56 scope) stays unmapped too, and `Option` does
    // not smuggle a mapping in.
    expect_error(
        "struct Row { #[bee(pk)] id: i64, at: NaiveTime }",
        "bee_orm: no SQL type mapping for `NaiveTime`: add #[bee(sql_type = \"...\")] to override",
    );
    expect_error(
        "struct Row { #[bee(pk)] id: i64, at: Option<DateTime<Local>> }",
        "bee_orm: no SQL type mapping for `DateTime<Local>`: add #[bee(sql_type = \"...\")] to override",
    );
}

#[test]
fn round9_crate_attribute_replaces_the_prefix() {
    // §61.2: the value is the ORM crate's path — for users who depend on
    // `bee_rust` and reach `bee_orm` through its re-export. Every emitted path
    // takes it, the const assertion and the `QuerySet` helper included.
    let expanded = expand_str(
        "#[bee(crate = \"bee_rust::bee_orm\")] \
         struct User { id: i64, name: String, #[bee(fk = Team)] team_id: i64 }",
    )
    .expect("must expand");
    assert!(expanded.contains("impl bee_rust :: bee_orm :: Model for User"));
    assert!(expanded.contains("bee_rust :: bee_orm :: QuerySet :: new (\"users\")"));
    assert!(
        expanded.contains("fn columns () -> & 'static [bee_rust :: bee_orm :: model :: ColumnDef]")
    );
    assert!(expanded.contains("bee_rust :: bee_orm :: Value :: from (self . id . clone ())"));
    assert!(expanded.contains(
        "const _ : fn () = || { fn assert_model < T : bee_rust :: bee_orm :: Model > () { } assert_model :: < Team > () ; } ;"
    ));
    // Every `bee_orm` in the output is part of the given prefix: nothing is
    // left on the default path.
    assert_eq!(
        expanded.matches("bee_orm").count(),
        expanded.matches("bee_rust :: bee_orm").count()
    );

    // Without the attribute the default path is untouched — the frozen
    // baselines elsewhere pin that byte-for-byte.
    let plain = expand_str("struct User { id: i64 }").expect("must expand");
    assert!(!plain.contains("bee_rust"));
}

#[test]
fn round9_crate_attribute_errors() {
    // §61.2: a bad crate path would mis-point every emitted path, so all
    // three value shapes fail loudly instead of expanding to code that
    // cannot build.
    expect_error(
        "#[bee(crate = 42)] struct User { id: i64 }",
        "bee_orm: #[bee(crate = \"path\")] expects a string literal",
    );
    expect_error(
        "#[bee(crate = \"\")] struct User { id: i64 }",
        "bee_orm: #[bee(crate = \"path\")] must not be empty",
    );
    expect_error(
        "#[bee(crate = \"bee_orm Model\")] struct User { id: i64 }",
        "bee_orm: #[bee(crate = \"path\")] is not a valid path",
    );
    expect_error(
        "#[bee(crate = \"bee_orm\")] #[bee(crate = \"bee_rust::bee_orm\")] struct User { id: i64 }",
        "bee_orm: duplicate #[bee(crate)] attribute",
    );
}
