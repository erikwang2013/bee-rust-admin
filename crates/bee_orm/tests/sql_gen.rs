// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;

#[derive(Model)]
#[bee(table = "sg_admin", pk = "id")]
pub struct SgAdmin {
    #[bee(auto)] pub id: u64,
    #[bee(unique)] pub username: String,
    pub status: i8,
    #[bee(index)] pub dept_id: u64,
    pub note: Option<String>,
    #[bee(text)] pub bio: String,
    pub created_at: chrono::NaiveDateTime,
}

#[test]
fn select_sql_keeps_order_limit_offset() {
    let sql = SgAdmin::query()
        .filter_eq("status", 1)
        .unwrap()
        .order_by("id DESC")
        .limit(10)
        .offset(20)
        .to_sql();
    assert_eq!(
        sql,
        "SELECT * FROM sg_admin WHERE status = ? ORDER BY id DESC LIMIT 10 OFFSET 20"
    );
}

#[test]
fn filter_in_builds_placeholders() {
    let qs = SgAdmin::query().filter_in("dept_id", vec![1u64, 2, 3]).unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM sg_admin WHERE dept_id IN (?, ?, ?)");
    assert_eq!(qs.params(), ["1", "2", "3"]);
}

#[test]
fn filter_in_empty_is_always_false() {
    let qs = SgAdmin::query().filter_in("dept_id", Vec::<u64>::new()).unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM sg_admin WHERE 1 = 0");
    assert!(qs.params().is_empty());
}

#[test]
fn filter_raw_binds_params() {
    let qs = SgAdmin::query().filter_raw("(status = ? OR username = ?)", &["1", "root"]);
    assert_eq!(qs.to_sql(), "SELECT * FROM sg_admin WHERE (status = ? OR username = ?)");
    assert_eq!(qs.params().len(), 2);
}

#[test]
fn page_is_one_based_and_capped() {
    assert_eq!(SgAdmin::query().page(3, 10).to_sql(), "SELECT * FROM sg_admin LIMIT 10 OFFSET 20");
    assert_eq!(SgAdmin::query().page(0, 9999).to_sql(), "SELECT * FROM sg_admin LIMIT 500 OFFSET 0");
}

#[test]
fn count_sql_drops_order_and_page() {
    let sql = SgAdmin::query()
        .filter_eq("status", 1)
        .unwrap()
        .order_by("id DESC")
        .limit(5)
        .count_sql();
    assert_eq!(sql, "SELECT COUNT(*) FROM sg_admin WHERE status = ?");
}

#[test]
fn filter_contains_escapes_like_wildcards() {
    // 用户输入里的 \ % _ 必须转义，否则搜「100%」会变成通配匹配
    let qs = SgAdmin::query().filter_contains("username", "100%_x\\y").unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM sg_admin WHERE username LIKE ?");
    assert_eq!(qs.params(), ["%100\\%\\_x\\\\y%"]);
}

#[test]
fn invalid_field_is_rejected() {
    assert!(SgAdmin::query().filter_eq("bad field", "x").is_err());
    assert!(SgAdmin::query().filter_in("bad;drop", vec![1u64]).is_err());
}

#[test]
fn create_table_ddl() {
    let ddl = bee_orm::syncdb::create_table_sql(&SgAdmin::META);
    assert!(ddl.contains("CREATE TABLE IF NOT EXISTS sg_admin"));
    assert!(ddl.contains("id BIGINT UNSIGNED AUTO_INCREMENT NOT NULL"));
    assert!(ddl.contains("PRIMARY KEY (id)"));
    assert!(ddl.contains("username VARCHAR(255) NOT NULL DEFAULT ''"));
    assert!(ddl.contains("UNIQUE KEY uk_sg_admin_username (username)"));
    assert!(ddl.contains("status TINYINT NOT NULL DEFAULT 0"));
    assert!(ddl.contains("note VARCHAR(255)")); // 可空，不加 NOT NULL
    assert!(!ddl.contains("note VARCHAR(255) NOT NULL"));
    assert!(ddl.contains("bio TEXT NOT NULL"));
    assert!(ddl.contains("KEY idx_sg_admin_dept_id (dept_id)"));
    assert!(ddl.contains("created_at DATETIME NOT NULL"));
    assert!(ddl.contains("ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"));
}

#[test]
fn add_column_ddl() {
    let ddl = bee_orm::syncdb::add_column_sql("sg_admin", &SgAdmin::META.columns[2]);
    assert_eq!(ddl, "ALTER TABLE sg_admin ADD COLUMN status TINYINT NOT NULL DEFAULT 0");
}
