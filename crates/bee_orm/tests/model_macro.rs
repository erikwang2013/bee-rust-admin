// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::{ColumnType, Model};

#[derive(Model)]
#[bee(table = "ut_admin", pk = "id")]
pub struct UtAdmin {
    #[bee(auto)] pub id: u64,
    #[bee(unique)] pub username: String,
    pub status: i8,
    #[bee(index)] pub dept_id: u64,
    pub note: Option<String>,
    #[bee(text)] pub bio: String,
    pub created_at: chrono::NaiveDateTime,
}

/// 无属性：表名取 snake_case + "s"（与旧行为兼容），主键自动认 `id`，不自增。
#[derive(Model)]
pub struct DefUser {
    pub id: i64,
    pub name: String,
}

/// 无 `id` 字段：无主键（连接表用）。
#[derive(Model)]
#[bee(table = "ut_link")]
pub struct UtLink {
    pub admin_id: u64,
    pub role_id: u64,
}

#[test]
fn meta_reflects_attributes() {
    assert_eq!(UtAdmin::META.table, "ut_admin");
    assert_eq!(UtAdmin::META.pk, Some("id"));
    assert_eq!(UtAdmin::META.columns.len(), 7);

    let id = UtAdmin::META.columns[0];
    assert_eq!(id.name, "id");
    assert!(id.auto);
    assert_eq!(id.ty, ColumnType::U64);

    assert!(UtAdmin::META.columns[1].unique);
    assert_eq!(UtAdmin::META.columns[2].ty, ColumnType::I8);
    assert!(UtAdmin::META.columns[3].index);
    assert!(UtAdmin::META.columns[4].nullable);
    assert!(UtAdmin::META.columns[5].text);
    assert_eq!(UtAdmin::META.columns[6].ty, ColumnType::DateTime);
}

#[test]
fn defaults_are_sane() {
    assert_eq!(DefUser::META.table, "def_users");
    assert_eq!(DefUser::META.pk, Some("id"));
    assert!(!DefUser::META.columns[0].auto);
    assert_eq!(DefUser::META.columns[0].ty, ColumnType::I64);

    assert_eq!(UtLink::META.table, "ut_link");
    assert_eq!(UtLink::META.pk, None);
}

#[test]
fn query_uses_table_name() {
    assert_eq!(DefUser::query().to_sql(), "SELECT * FROM def_users");
}
