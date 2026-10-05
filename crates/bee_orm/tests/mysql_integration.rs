// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 真库集成测试：设了 BEE_ORM_TEST_DSN 才跑，否则跳过（打印原因）。
use bee_orm::{Db, Model, OrmError, SyncdbMode};

const DDL: &str = "CREATE TABLE it_admin (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  username VARCHAR(255) NOT NULL DEFAULT '',
  password VARCHAR(255) NOT NULL DEFAULT '',
  status TINYINT NOT NULL DEFAULT 0,
  dept_id BIGINT UNSIGNED NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL,
  UNIQUE KEY uk_username (username)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";

#[derive(Model)]
#[bee(table = "it_admin", pk = "id")]
pub struct ItAdmin {
    #[bee(auto)] pub id: u64,
    pub username: String,
    pub password: String,
    pub status: i8,
    pub dept_id: u64,
    pub created_at: chrono::NaiveDateTime,
}

/// 事务测试用独立表，避免与 `crud_and_query_flow` 并发抢同一张表。
#[derive(Model)]
#[bee(table = "it_tx_admin", pk = "id")]
pub struct ItTxAdmin {
    #[bee(auto)] pub id: u64,
    pub username: String,
    pub password: String,
    pub status: i8,
    pub dept_id: u64,
    pub created_at: chrono::NaiveDateTime,
}

fn dsn() -> Option<String> {
    match std::env::var("BEE_ORM_TEST_DSN") {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("跳过：未设置 BEE_ORM_TEST_DSN");
            None
        }
    }
}

fn new_admin(name: &str, status: i8, dept: u64) -> ItAdmin {
    ItAdmin {
        id: 0,
        username: name.into(),
        password: "x".into(),
        status,
        dept_id: dept,
        created_at: chrono::Utc::now().naive_utc(),
    }
}

#[tokio::test]
async fn crud_and_query_flow() {
    let Some(dsn) = dsn() else { return };
    let db = Db::connect(&dsn).await.unwrap();
    db.exec_sql("DROP TABLE IF EXISTS it_admin").await.unwrap();
    db.exec_sql(DDL).await.unwrap();

    // insert 回填自增主键
    let mut a = new_admin("alice", 1, 7);
    let id = db.insert(&mut a).await.unwrap();
    assert!(id > 0);
    assert_eq!(a.id, id);

    // read
    let got = db.read::<ItAdmin>(id).await.unwrap().unwrap();
    assert_eq!(got.username, "alice");
    assert_eq!(got.dept_id, 7);
    assert!(db.read::<ItAdmin>(999_999).await.unwrap().is_none());

    // update
    let mut got = got;
    got.status = 0;
    assert_eq!(db.update(&got).await.unwrap(), 1);
    assert_eq!(db.read::<ItAdmin>(id).await.unwrap().unwrap().status, 0);

    // 唯一键冲突 → DuplicateKey
    let mut dup = new_admin("alice", 1, 7);
    assert!(matches!(db.insert(&mut dup).await, Err(OrmError::DuplicateKey(_))));

    // 再插两条，查过滤/分页/count
    for n in ["bob", "carol"] {
        let mut u = new_admin(n, 1, 7);
        db.insert(&mut u).await.unwrap();
    }
    assert_eq!(ItAdmin::query().count(&db).await.unwrap(), 3);
    assert_eq!(ItAdmin::query().filter_eq("status", 1).unwrap().count(&db).await.unwrap(), 2);
    let all = ItAdmin::query().order_by("id ASC").fetch_all(&db).await.unwrap();
    assert_eq!(all.len(), 3);
    let (rows, total) = ItAdmin::query().filter_eq("status", 1).unwrap().fetch_page(&db, 1, 1).await.unwrap();
    assert_eq!((rows.len(), total), (1, 2));
    let one = ItAdmin::query().filter_eq("username", "bob").unwrap().fetch_one(&db).await.unwrap().unwrap();
    assert_eq!(one.username, "bob");
    assert!(ItAdmin::query().filter_eq("username", "nobody").unwrap().fetch_one(&db).await.unwrap().is_none());
    assert_eq!(ItAdmin::query().filter_in("dept_id", vec![7u64]).unwrap().count(&db).await.unwrap(), 3);
    assert_eq!(ItAdmin::query().filter_in("dept_id", Vec::<u64>::new()).unwrap().count(&db).await.unwrap(), 0);
    assert_eq!(ItAdmin::query().filter_contains("username", "ar").unwrap().count(&db).await.unwrap(), 1);

    // delete
    assert_eq!(db.delete::<ItAdmin>(id).await.unwrap(), 1);
    assert!(db.read::<ItAdmin>(id).await.unwrap().is_none());

    db.exec_sql("DROP TABLE it_admin").await.unwrap();
}

#[tokio::test]
async fn tx_commit_and_rollback() {
    let Some(dsn) = dsn() else { return };
    let db = Db::connect(&dsn).await.unwrap();
    db.exec_sql("DROP TABLE IF EXISTS it_tx_admin").await.unwrap();
    db.exec_sql(&DDL.replacen("it_admin", "it_tx_admin", 1)).await.unwrap();

    // commit 后落库
    let mut tx = db.begin().await.unwrap();
    let mut a = ItTxAdmin { id: 0, username: "tx_alice".into(), password: "x".into(), status: 1, dept_id: 7, created_at: chrono::Utc::now().naive_utc() };
    let id = tx.insert(&mut a).await.unwrap();
    assert!(id > 0);
    assert!(tx.read::<ItTxAdmin>(id).await.unwrap().is_some());
    tx.commit().await.unwrap();
    assert_eq!(db.read::<ItTxAdmin>(id).await.unwrap().unwrap().username, "tx_alice");

    // rollback 后不可见（事务内可见，事务外没有）
    let mut tx = db.begin().await.unwrap();
    let mut b = ItTxAdmin { id: 0, username: "tx_bob".into(), password: "x".into(), status: 1, dept_id: 7, created_at: chrono::Utc::now().naive_utc() };
    let bid = tx.insert(&mut b).await.unwrap();
    assert!(tx.read::<ItTxAdmin>(bid).await.unwrap().is_some());
    tx.rollback().await.unwrap();
    assert!(db.read::<ItTxAdmin>(bid).await.unwrap().is_none());

    // 事务内 update/delete 同样生效
    let mut tx = db.begin().await.unwrap();
    let mut got = tx.read::<ItTxAdmin>(id).await.unwrap().unwrap();
    got.status = 0;
    assert_eq!(tx.update(&got).await.unwrap(), 1);
    assert_eq!(tx.delete::<ItTxAdmin>(id).await.unwrap(), 1);
    tx.commit().await.unwrap();
    assert!(db.read::<ItTxAdmin>(id).await.unwrap().is_none());

    db.exec_sql("DROP TABLE it_tx_admin").await.unwrap();
}

/// 连接表：无主键、无自增。
#[derive(Model)]
#[bee(table = "it_admin_role")]
pub struct ItAdminRole {
    pub admin_id: u64,
    pub role_id: u64,
}

#[tokio::test]
async fn relations_flow() {
    let Some(dsn) = dsn() else { return };
    let db = Db::connect(&dsn).await.unwrap();
    assert_eq!(ItAdminRole::META.pk, None);
    db.exec_sql("DROP TABLE IF EXISTS it_admin_role").await.unwrap();
    db.exec_sql(
        "CREATE TABLE it_admin_role (
           admin_id BIGINT UNSIGNED NOT NULL,
           role_id BIGINT UNSIGNED NOT NULL,
           PRIMARY KEY (admin_id, role_id)
         ) ENGINE=InnoDB",
    )
    .await
    .unwrap();

    db.set_relations("it_admin_role", ("admin_id", 1), "role_id", &[10, 20, 30]).await.unwrap();
    let mut got = db.get_relations("it_admin_role", ("admin_id", 1), "role_id").await.unwrap();
    got.sort();
    assert_eq!(got, vec![10, 20, 30]);

    // set = 删旧插新（事务内）
    db.set_relations("it_admin_role", ("admin_id", 1), "role_id", &[20, 40]).await.unwrap();
    let mut got = db.get_relations("it_admin_role", ("admin_id", 1), "role_id").await.unwrap();
    got.sort();
    assert_eq!(got, vec![20, 40]);

    // 空集合 = 清空
    db.set_relations("it_admin_role", ("admin_id", 1), "role_id", &[]).await.unwrap();
    assert!(db.get_relations("it_admin_role", ("admin_id", 1), "role_id").await.unwrap().is_empty());

    // 其他 owner 不受影响
    db.set_relations("it_admin_role", ("admin_id", 2), "role_id", &[10]).await.unwrap();
    assert_eq!(db.del_relations("it_admin_role", "admin_id", 2).await.unwrap(), 1);

    // 重复 id → 唯一键冲突 → 整个事务回滚，旧关联不被清空（防误清）
    db.set_relations("it_admin_role", ("admin_id", 3), "role_id", &[10, 20]).await.unwrap();
    assert!(matches!(
        db.set_relations("it_admin_role", ("admin_id", 3), "role_id", &[10, 10]).await,
        Err(OrmError::DuplicateKey(_))
    ));
    let mut got = db.get_relations("it_admin_role", ("admin_id", 3), "role_id").await.unwrap();
    got.sort();
    assert_eq!(got, vec![10, 20]);

    // 非法标识符拒绝
    assert!(matches!(
        db.set_relations("it_admin_role; DROP TABLE x", ("admin_id", 1), "role_id", &[1]).await,
        Err(OrmError::InvalidField(_))
    ));

    db.exec_sql("DROP TABLE it_admin_role").await.unwrap();
}

#[derive(Model)]
#[bee(table = "it_sync", pk = "id")]
pub struct ItSyncV1 {
    #[bee(auto)] pub id: u64,
    #[bee(unique)] pub name: String,
    pub status: i8,
}

#[derive(Model)]
#[bee(table = "it_sync", pk = "id")]
pub struct ItSyncV2 {
    #[bee(auto)] pub id: u64,
    #[bee(unique)] pub name: String,
    pub status: i8,
    pub remark: String,
    #[bee(index)] pub dept_id: u64,
}

#[tokio::test]
async fn syncdb_creates_alters_and_is_idempotent() {
    let Some(dsn) = dsn() else { return };
    let db = Db::connect(&dsn).await.unwrap();
    db.exec_sql("DROP TABLE IF EXISTS it_sync").await.unwrap();

    let ddl = db.syncdb(&[ItSyncV1::META], SyncdbMode::Safe).await.unwrap();
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].starts_with("CREATE TABLE IF NOT EXISTS it_sync"));

    // 幂等：再跑一次不产生 DDL
    assert!(db.syncdb(&[ItSyncV1::META], SyncdbMode::Safe).await.unwrap().is_empty());

    // 模型加列 → ALTER 补列 + 补索引
    let ddl = db.syncdb(&[ItSyncV2::META], SyncdbMode::Safe).await.unwrap();
    assert!(ddl.iter().any(|d| d.contains("ADD COLUMN remark VARCHAR(255) NOT NULL DEFAULT ''")));
    assert!(ddl.iter().any(|d| d.contains("CREATE INDEX idx_it_sync_dept_id")));

    // 补列补索引之后同样幂等（每次启动都会跑 syncdb）
    assert!(db.syncdb(&[ItSyncV2::META], SyncdbMode::Safe).await.unwrap().is_empty());

    // 新列可用
    let mut r = ItSyncV2 { id: 0, name: "x".into(), status: 1, remark: "hi".into(), dept_id: 3 };
    assert!(db.insert(&mut r).await.unwrap() > 0);

    // Force 未实现
    assert!(matches!(
        db.syncdb(&[ItSyncV1::META], SyncdbMode::Force).await,
        Err(OrmError::Unsupported(_))
    ));

    db.exec_sql("DROP TABLE it_sync").await.unwrap();
}
