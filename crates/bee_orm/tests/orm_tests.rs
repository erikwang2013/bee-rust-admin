// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::{Model, Value};

#[derive(Model)]
#[allow(dead_code)]
struct User {
    id: i32,
    name: String,
    age: i32,
}

/// Exercises every `#[bee(...)]` attribute at once: table rename, column
/// rename, an explicit auto primary key and an ignored (non-column) field.
#[derive(Model, Debug, PartialEq)]
#[bee(table = "user_accounts")]
#[allow(dead_code)]
struct Account {
    #[bee(pk, auto)]
    id: i64,
    #[bee(column = "user_name")]
    name: String,
    age: Option<i32>,
    #[bee(ignore)]
    avatar_cache: Vec<u8>,
}

#[test]
fn test_table_name() {
    assert_eq!(User::table_name(), "users");
}

#[test]
fn test_model_trait_surface() {
    assert_eq!(<User as Model>::table_name(), "users");
    assert_eq!(<User as Model>::pk_column(), "id");
}

#[test]
fn test_query_select_all() {
    let sql = User::query().to_sql();
    assert_eq!(sql, "SELECT * FROM users");
}

#[test]
fn test_query_with_filter() {
    let sql = User::query().filter("age > 18").to_sql();
    assert_eq!(sql, "SELECT * FROM users WHERE age > 18");
}

#[test]
fn test_query_with_multiple_filters() {
    let sql = User::query().filter("age > 18").filter("name LIKE 'A%'").to_sql();
    assert_eq!(sql, "SELECT * FROM users WHERE age > 18 AND name LIKE 'A%'");
}

#[test]
fn test_query_with_order_by() {
    let sql = User::query().order_by("id DESC").to_sql();
    assert_eq!(sql, "SELECT * FROM users ORDER BY id DESC");
}

#[test]
fn test_query_with_limit_offset() {
    let sql = User::query().limit(10).offset(20).to_sql();
    assert_eq!(sql, "SELECT * FROM users LIMIT 10 OFFSET 20");
}

#[test]
fn test_query_combined() {
    let sql = User::query().filter("age > 18").order_by("id DESC").limit(10).offset(5).to_sql();
    assert_eq!(sql, "SELECT * FROM users WHERE age > 18 ORDER BY id DESC LIMIT 10 OFFSET 5");
}

#[test]
fn test_filter_eq_parametrised() {
    let qs = User::query().filter_eq("name", "o'neil").unwrap();
    let sql = qs.to_sql();
    assert_eq!(sql, "SELECT * FROM users WHERE name = ?");
    assert_eq!(qs.params(), &[Value::from("o'neil")]);
    assert!(!sql.contains("o'neil"));
}

#[test]
fn test_filter_comparisons() {
    let qs = User::query().filter_gt("age", 18).unwrap().filter_lt("age", 65).unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM users WHERE age > ? AND age < ?");
    assert_eq!(qs.params(), &[Value::Int(18), Value::Int(65)]);
}

#[test]
fn test_filter_contains_parametrised() {
    let qs = User::query().filter_contains("name", "o'neil").unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM users WHERE name LIKE ?");
    assert_eq!(qs.params(), &[Value::from("%o'neil%")]);
    assert!(!qs.to_sql().contains("o'neil"));
}

#[test]
fn test_mixed_raw_and_parametrised_filters() {
    let qs = User::query()
        .filter("active = 1")
        .filter_eq("name", "o'neil")
        .unwrap()
        .filter_gt("age", 18)
        .unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM users WHERE active = 1 AND name = ? AND age > ?");
    assert_eq!(qs.params(), &[Value::from("o'neil"), Value::Int(18)]);
}

#[test]
fn test_invalid_field_name_rejected() {
    for bad in ["name; DROP TABLE users", "na me", "age DESC --", "", "1name"] {
        assert!(matches!(
            User::query().filter_eq(bad, "x"),
            Err(bee_orm::OrmError::InvalidField(_))
        ));
        assert!(matches!(
            User::query().filter_gt(bad, "x"),
            Err(bee_orm::OrmError::InvalidField(_))
        ));
        assert!(matches!(
            User::query().filter_contains(bad, "x"),
            Err(bee_orm::OrmError::InvalidField(_))
        ));
    }
}

#[test]
fn test_attribute_model_metadata() {
    assert_eq!(Account::table_name(), "user_accounts");
    assert_eq!(<Account as Model>::table_name(), "user_accounts");
    assert_eq!(<Account as Model>::pk_column(), "id");
}

fn account() -> Account {
    Account { id: 7, name: "alice".into(), age: Some(30), avatar_cache: vec![9] }
}

#[test]
fn test_attribute_model_write_values() {
    let account = account();
    // `auto` and `ignore` are skipped on insert, the pk on update; `column`
    // renames the column; a `None` value binds as null.
    assert_eq!(
        account.insert_values(),
        vec![("user_name", Value::from("alice")), ("age", Value::Int(30))]
    );
    assert_eq!(
        account.update_values(),
        vec![("user_name", Value::from("alice")), ("age", Value::Int(30))]
    );
    assert_eq!(account.pk_value(), Value::Int(7));

    let anonymous = Account { id: 8, name: "anon".into(), age: None, avatar_cache: vec![] };
    assert_eq!(
        anonymous.insert_values(),
        vec![("user_name", Value::from("anon")), ("age", Value::Null)]
    );
}

#[test]
fn test_pk_written_on_insert_but_not_update() {
    let user = User { id: 1, name: "alice".into(), age: 30 };
    assert_eq!(
        user.insert_values(),
        vec![("id", Value::Int(1)), ("name", Value::from("alice")), ("age", Value::Int(30))]
    );
    assert_eq!(user.update_values(), vec![("name", Value::from("alice")), ("age", Value::Int(30))]);
    assert_eq!(user.pk_value(), Value::Int(1));
}

#[test]
fn test_attribute_model_from_row() {
    let mut row = bee_orm::Row::new();
    row.insert("id".into(), 7.into());
    row.insert("user_name".into(), "alice".into());
    row.insert("age".into(), 30.into());

    let decoded = Account::from_row(&row).unwrap();
    let expected = Account { id: 7, name: "alice".into(), age: Some(30), avatar_cache: Vec::new() };
    assert_eq!(decoded, expected);

    // A missing optional column decodes as `None`; the ignored field takes
    // `Default` instead of being read from the row.
    let mut row = bee_orm::Row::new();
    row.insert("id".into(), 7.into());
    row.insert("user_name".into(), "alice".into());
    let decoded = Account::from_row(&row).unwrap();
    assert_eq!(decoded.age, None);
    assert_eq!(decoded.avatar_cache, Vec::<u8>::new());
}

#[test]
fn test_from_row_errors_instead_of_panicking() {
    // Missing non-optional column: JSON null where `String` is expected.
    let mut row = bee_orm::Row::new();
    row.insert("id".into(), 7.into());
    assert!(matches!(
        Account::from_row(&row),
        Err(bee_orm::OrmError::QueryError(message)) if message.contains("user_name")
    ));

    // Wrong type: a text value where `i64` is expected.
    let mut row = bee_orm::Row::new();
    row.insert("id".into(), "seven".into());
    row.insert("user_name".into(), "alice".into());
    assert!(Account::from_row(&row).is_err());
}
