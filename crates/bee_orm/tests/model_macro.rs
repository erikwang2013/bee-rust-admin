// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::__private::sqlx::Execute;
use bee_orm::__private::{MySql, QueryBuilder};
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

fn sample_admin() -> UtAdmin {
    UtAdmin {
        id: 7,
        username: "bob".into(),
        status: 1,
        dept_id: 3,
        note: None,
        bio: "hi".into(),
        created_at: chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc(),
    }
}

/// 自增列不参与 INSERT 绑定，逗号只在值之间（sqlx 的 `separated` 不留尾逗号）。
#[test]
fn insert_binds_skip_auto_pk() {
    let qb: QueryBuilder<MySql> = QueryBuilder::new(
        "INSERT INTO ut_admin (username, status, dept_id, note, bio, created_at) VALUES (",
    );
    let mut qb = sample_admin().bind_insert(qb);
    let q = qb.push(")").build();
    assert_eq!(
        q.sql(),
        "INSERT INTO ut_admin (username, status, dept_id, note, bio, created_at) VALUES (?, ?, ?, ?, ?, ?)"
    );
}

/// UPDATE 绑定全部非自增列，并以主键列收尾的 WHERE 结束。
#[test]
fn update_binds_set_all_then_where_pk() {
    let qb: QueryBuilder<MySql> = QueryBuilder::new("UPDATE ut_admin ");
    let mut qb = sample_admin().bind_update(qb);
    assert_eq!(
        qb.build().sql(),
        "UPDATE ut_admin SET username = ?, status = ?, dept_id = ?, note = ?, bio = ?, created_at = ? WHERE id = ?"
    );
}

/// 无主键模型：不生成 WHERE（Db::update 会先拒绝这种模型）。
#[test]
fn update_without_pk_has_no_where() {
    let link = UtLink { admin_id: 1, role_id: 2 };
    let qb: QueryBuilder<MySql> = QueryBuilder::new("UPDATE ut_link ");
    let mut qb = link.bind_update(qb);
    assert_eq!(qb.build().sql(), "UPDATE ut_link SET admin_id = ?, role_id = ?");
}
