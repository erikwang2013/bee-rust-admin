// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "sqlite")]
//! End-to-end acceptance tests against an in-memory SQLite database.
use bee_orm::pool::sqlite::Pool;
use bee_orm::{Db, Model, Value};

#[derive(Model, Debug, Clone, PartialEq)]
struct User {
    #[bee(pk, auto)]
    id: i64,
    name: String,
    age: Option<i32>,
    active: bool,
    data: Vec<u8>,
}

/// `:memory:` pool with the `users` table created.
async fn setup() -> Pool {
    let pool = Pool::connect(":memory:", 4).unwrap();
    pool.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, \
         age INTEGER, active INTEGER, data BLOB)",
        &[],
    )
    .await
    .unwrap();
    pool
}

fn alice() -> User {
    User { id: 0, name: "alice".into(), age: Some(30), active: true, data: vec![1, 2, 3] }
}

fn bob() -> User {
    User { id: 0, name: "bob".into(), age: Some(16), active: false, data: vec![] }
}

/// Insert `user` and return the database-assigned primary key.
async fn insert_user(pool: &Pool, user: &User) -> i64 {
    assert_eq!(user.insert(pool).await.unwrap(), 1);
    User::query()
        .filter_eq("name", user.name.as_str())
        .unwrap()
        .one(pool)
        .await
        .unwrap()
        .unwrap()
        .id
}

#[tokio::test]
async fn insert_skips_the_auto_pk() {
    let pool = setup().await;
    let user = alice();
    // The struct carries id 0, but `auto` keeps it out of the INSERT, so the
    // database assigns the key itself.
    assert!(!user.insert_values().iter().any(|(column, _)| *column == "id"));
    assert_eq!(user.insert(&pool).await.unwrap(), 1);

    let stored = User::query().one(&pool).await.unwrap().unwrap();
    assert_eq!(stored.id, 1);
    assert_eq!(stored.name, "alice");
}

#[tokio::test]
async fn one_roundtrips_every_field() {
    let pool = setup().await;
    let id = insert_user(&pool, &alice()).await;

    let found =
        User::query().filter_eq("name", "alice").unwrap().one(&pool).await.unwrap().unwrap();
    assert_ne!(found.id, 0);
    assert_eq!(found, User { id, ..alice() });
    assert!(found.active); // stored as the integer 1, decoded back to `true`
    assert_eq!(found.data, vec![1, 2, 3]);
}

#[tokio::test]
async fn option_field_roundtrips_null() {
    let pool = setup().await;
    let carol = User { id: 0, name: "carol".into(), age: None, active: false, data: vec![] };
    insert_user(&pool, &carol).await;

    let rows =
        pool.query("SELECT age FROM users WHERE name = ?", &[Value::from("carol")]).await.unwrap();
    assert!(rows[0]["age"].is_null());

    let found =
        User::query().filter_eq("name", "carol").unwrap().one(&pool).await.unwrap().unwrap();
    assert_eq!(found.age, None);
}

#[tokio::test]
async fn one_returns_none_without_a_match() {
    let pool = setup().await;
    insert_user(&pool, &alice()).await;

    let found = User::query().filter_eq("name", "nobody").unwrap().one(&pool).await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn count_and_exists_with_typed_filters() {
    let pool = setup().await;
    insert_user(&pool, &alice()).await;
    insert_user(&pool, &bob()).await;

    let matching = User::query().filter_gt("age", 18).unwrap();
    assert_eq!(matching.count(&pool).await.unwrap(), 1);
    assert!(matching.exists(&pool).await.unwrap());

    let none = User::query().filter_gt("age", 100).unwrap();
    assert_eq!(none.count(&pool).await.unwrap(), 0);
    assert!(!none.exists(&pool).await.unwrap());
}

#[tokio::test]
async fn all_with_order_by() {
    let pool = setup().await;
    let alice_id = insert_user(&pool, &alice()).await;
    let bob_id = insert_user(&pool, &bob()).await;

    let users = User::query().order_by("id").all(&pool).await.unwrap();
    assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), vec![alice_id, bob_id]);
    assert_eq!(
        users.iter().map(|user| user.name.as_str()).collect::<Vec<_>>(),
        vec!["alice", "bob"]
    );

    let reversed = User::query().order_by("id DESC").all(&pool).await.unwrap();
    assert_eq!(
        reversed.iter().map(|user| user.name.as_str()).collect::<Vec<_>>(),
        vec!["bob", "alice"]
    );
}

#[tokio::test]
async fn model_update_is_scoped_to_the_pk() {
    let pool = setup().await;
    let alice_id = insert_user(&pool, &alice()).await;
    let bob_id = insert_user(&pool, &bob()).await;

    let mut found =
        User::query().filter_eq("id", alice_id).unwrap().one(&pool).await.unwrap().unwrap();
    found.name = "alicia".into();
    found.age = Some(31);
    found.active = false;
    assert_eq!(found.update(&pool).await.unwrap(), 1);

    let reread =
        User::query().filter_eq("id", alice_id).unwrap().one(&pool).await.unwrap().unwrap();
    assert_eq!(reread, found);

    // The WHERE is the pk, so the other row is untouched.
    let bob_row = User::query().filter_eq("id", bob_id).unwrap().one(&pool).await.unwrap().unwrap();
    assert_eq!(bob_row, User { id: bob_id, ..bob() });
}

#[tokio::test]
async fn queryset_update_binds_set_before_where() {
    let pool = setup().await;
    let alice_id = insert_user(&pool, &alice()).await;
    let bob_id = insert_user(&pool, &bob()).await;

    let affected = User::query()
        .filter_eq("id", bob_id)
        .unwrap()
        .update(&pool, &[("name", "bobby".into())])
        .await
        .unwrap();
    // A flipped parameter order would bind `bobby` to `id` and the id to
    // `name`, matching no row at all.
    assert_eq!(affected, 1);

    let bob_row = User::query().filter_eq("id", bob_id).unwrap().one(&pool).await.unwrap().unwrap();
    assert_eq!(bob_row.name, "bobby");
    let alice_row =
        User::query().filter_eq("id", alice_id).unwrap().one(&pool).await.unwrap().unwrap();
    assert_eq!(alice_row.name, "alice");
}

#[tokio::test]
async fn deletes_by_model_and_queryset() {
    let pool = setup().await;
    let alice_id = insert_user(&pool, &alice()).await;
    let bob_id = insert_user(&pool, &bob()).await;
    assert_eq!(User::query().count(&pool).await.unwrap(), 2);

    let found = User::query().filter_eq("id", alice_id).unwrap().one(&pool).await.unwrap().unwrap();
    assert_eq!(found.delete(&pool).await.unwrap(), 1);
    assert_eq!(User::query().count(&pool).await.unwrap(), 1);

    assert_eq!(User::query().filter_eq("id", bob_id).unwrap().delete(&pool).await.unwrap(), 1);
    assert_eq!(User::query().count(&pool).await.unwrap(), 0);

    // Unfiltered delete on an emptied table touches nothing.
    assert_eq!(User::query().delete(&pool).await.unwrap(), 0);
}

#[tokio::test]
async fn dyn_db_is_object_safe() {
    let pool = setup().await;
    insert_user(&pool, &alice()).await;
    insert_user(&pool, &bob()).await;

    let db: &dyn Db = &pool;
    assert_eq!(User::query().all(db).await.unwrap().len(), 2);
    assert_eq!(User::query().filter_eq("name", "alice").unwrap().count(db).await.unwrap(), 1);
    assert!(User::query().filter_eq("name", "alice").unwrap().exists(db).await.unwrap());
    let found = User::query().filter_eq("name", "alice").unwrap().one(db).await.unwrap().unwrap();
    assert_eq!(found.name, "alice");
}

#[tokio::test]
async fn bound_values_keep_their_type() {
    let pool = setup().await;
    pool.execute("CREATE TABLE bindings (flag INTEGER, f REAL, t TEXT, n TEXT, b BLOB)", &[])
        .await
        .unwrap();
    pool.execute(
        "INSERT INTO bindings (flag, f, t, n, b) VALUES (?, ?, ?, ?, ?)",
        &[
            Value::Bool(true),
            Value::from(1.5f64),
            Value::Text("text".into()),
            Value::Null,
            Value::Bytes(vec![1, 2, 3]),
        ],
    )
    .await
    .unwrap();

    let rows = pool.query("SELECT flag, f, t, n, b FROM bindings", &[]).await.unwrap();
    assert_eq!(rows[0]["flag"], 1);
    assert!(bee_orm::decode::<bool>(&rows[0], "flag").unwrap());
    assert_eq!(rows[0]["f"], 1.5);
    assert_eq!(rows[0]["t"], "text");
    assert!(rows[0]["n"].is_null());
    // A BLOB comes back as a JSON array of byte numbers.
    assert!(rows[0]["b"].is_array());
    assert_eq!(bee_orm::decode::<Vec<u8>>(&rows[0], "b").unwrap(), vec![1, 2, 3]);
}
