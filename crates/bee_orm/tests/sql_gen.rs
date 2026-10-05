// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;

#[derive(Model)]
#[bee(table = "sg_admin", pk = "id")]
pub struct SgAdmin {
    #[bee(auto)] pub id: u64,
    #[bee(unique)] pub username: String,
    pub status: i8,
    pub dept_id: u64,
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
fn invalid_field_is_rejected() {
    assert!(SgAdmin::query().filter_eq("bad field", "x").is_err());
    assert!(SgAdmin::query().filter_in("bad;drop", vec![1u64]).is_err());
}
