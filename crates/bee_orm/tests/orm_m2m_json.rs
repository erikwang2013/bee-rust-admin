// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "sqlite")]
//! Round-5 acceptance on in-memory SQLite: many-to-many relations (§43,
//! join-table DDL, attach/detach, `related*`) and JSON columns (§45,
//! round-trip, raw `Value::Json`, invalid TEXT).
use bee_orm::pool::sqlite::Pool;
use bee_orm::{Model, OrmError, QuerySet, Value, decode, m2m, migrate};
use serde_json::json;

fn pool() -> Pool {
    // The documented single-connection clamp is what makes `:memory:` coherent:
    // every checkout sees the same database.
    Pool::connect(":memory:", 1).unwrap()
}

// ---------------------------------------------------------------------- m2m

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(m2m(Tag))]
struct User {
    #[bee(pk, auto)]
    id: i64,
    name: String,
}

#[derive(Model, Debug, Clone, PartialEq)]
struct Tag {
    #[bee(pk, auto)]
    id: i64,
    label: String,
    #[bee(soft_delete)]
    deleted: bool,
}

async fn user(pool_: &Pool, name: &str) -> User {
    let user = User { id: 0, name: name.into() };
    user.insert(pool_).await.unwrap();
    QuerySet::<User>::new(User::table_name())
        .filter_eq("name", name)
        .unwrap()
        .one(pool_)
        .await
        .unwrap()
        .unwrap()
}

async fn tag(pool_: &Pool, label: &str) -> Tag {
    let tag = Tag { id: 0, label: label.into(), deleted: false };
    tag.insert(pool_).await.unwrap();
    QuerySet::<Tag>::new(Tag::table_name())
        .filter_eq("label", label)
        .unwrap()
        .one(pool_)
        .await
        .unwrap()
        .unwrap()
}

/// The target first (the join table references both sides), then the
/// declaring model; the join DDL is the §43 pinned shape.
async fn schema(pool_: &Pool) {
    migrate::create_table::<Tag, _>(pool_).await.unwrap();
    migrate::create_table::<User, _>(pool_).await.unwrap();
    let rows = pool_
        .query("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'user_tag'", &[])
        .await
        .unwrap();
    let ddl = decode::<String>(&rows[0], "sql").unwrap();
    assert!(ddl.contains("user_id INTEGER NOT NULL REFERENCES users (id)"), "{ddl}");
    assert!(ddl.contains("tag_id INTEGER NOT NULL REFERENCES tags (id)"), "{ddl}");
    assert!(ddl.contains("PRIMARY KEY (user_id, tag_id)"), "{ddl}");
    assert!(ddl.contains("UNIQUE (tag_id, user_id)"), "{ddl}");
}

#[tokio::test]
async fn attach_related_detach_shrinks_and_detach_is_idempotent() {
    let pool_ = pool();
    schema(&pool_).await;
    let user = user(&pool_, "ada").await;
    let rust = tag(&pool_, "rust").await;
    let db = tag(&pool_, "db").await;

    assert_eq!(m2m::attach(&pool_, &user, &rust).await.unwrap(), 1);
    assert_eq!(m2m::attach(&pool_, &user, &db).await.unwrap(), 1);

    let ids = m2m::related_ids::<User, Tag, _>(&pool_, &user).await.unwrap();
    assert_eq!(ids.len(), 2);
    let related = m2m::related::<User, Tag, _>(&pool_, &user).await.unwrap();
    assert_eq!(related.len(), 2);

    // A duplicate attach is the join table's composite primary key, not a
    // silent no-op.
    let err = m2m::attach(&pool_, &user, &rust).await.err().unwrap();
    assert!(matches!(err, OrmError::QueryError(ref msg) if msg.contains("UNIQUE")), "{err}");

    assert_eq!(m2m::detach(&pool_, &user, &rust).await.unwrap(), 1);
    let related = m2m::related::<User, Tag, _>(&pool_, &user).await.unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].label, "db");

    // Second detach affects 0 rows.
    assert_eq!(m2m::detach(&pool_, &user, &rust).await.unwrap(), 0);
}

#[tokio::test]
async fn related_for_is_index_aligned_with_an_empty_group() {
    let pool_ = pool();
    schema(&pool_).await;
    let ada = user(&pool_, "ada").await;
    let bob = user(&pool_, "bob").await;
    let rust = tag(&pool_, "rust").await;
    m2m::attach(&pool_, &ada, &rust).await.unwrap();

    // ada twice (aligned groups, not deduped away), bob with no tags.
    let groups = m2m::related_for::<User, Tag, _>(&pool_, &[ada.clone(), bob.clone(), ada.clone()])
        .await
        .unwrap();
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].len(), 1);
    assert_eq!(groups[0][0].label, "rust");
    assert_eq!(groups[1].len(), 0);
    assert_eq!(groups[2].len(), 1);
}

#[tokio::test]
async fn soft_deleted_targets_are_hidden_but_join_rows_stay() {
    let pool_ = pool();
    schema(&pool_).await;
    let user = user(&pool_, "ada").await;
    let rust = tag(&pool_, "rust").await;
    m2m::attach(&pool_, &user, &rust).await.unwrap();
    rust.delete(&pool_).await.unwrap();

    assert_eq!(m2m::related::<User, Tag, _>(&pool_, &user).await.unwrap().len(), 0);
    let groups =
        m2m::related_for::<User, Tag, _>(&pool_, std::slice::from_ref(&user)).await.unwrap();
    assert_eq!(groups[0].len(), 0);

    // The escape hatch still sees the id (and `with_deleted` the row).
    let ids = m2m::related_ids::<User, Tag, _>(&pool_, &user).await.unwrap();
    assert_eq!(ids.len(), 1);
    let all = QuerySet::<Tag>::new(Tag::table_name()).with_deleted().all(&pool_).await.unwrap();
    assert_eq!(all.len(), 1);

    // No relation at all is a loud error, not an empty list.
    let err = m2m::related::<Tag, User, _>(&pool_, &rust).await.err().unwrap();
    assert!(matches!(err, OrmError::QueryError(ref msg) if msg.contains("m2m")), "{err}");
}

// --------------------------------------------------------------------- json

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "docs")]
struct Doc {
    #[bee(pk, auto)]
    id: i64,
    payload: serde_json::Value,
    extra: Option<serde_json::Value>,
}

#[tokio::test]
async fn json_columns_round_trip_object_string_and_null() {
    let pool_ = pool();
    migrate::create_table::<Doc, _>(&pool_).await.unwrap();
    let rows = pool_
        .query("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'docs'", &[])
        .await
        .unwrap();
    let ddl = decode::<String>(&rows[0], "sql").unwrap();
    assert!(ddl.contains("payload TEXT NOT NULL"), "json renders TEXT on sqlite: {ddl}");

    let object = json!({ "k": [1, true, "x"] });
    let doc = Doc { id: 0, payload: object.clone(), extra: Some(json!(null)) };
    doc.insert(&pool_).await.unwrap();

    let doc = Doc { id: 0, payload: json!("just a string"), extra: None };
    doc.insert(&pool_).await.unwrap();

    let docs = QuerySet::<Doc>::new("docs").order_by("id").all(&pool_).await.unwrap();
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].payload, object, "object round-trips");
    assert_eq!(docs[0].extra, Some(json!(null)), "a stored JSON null is Some(null)");
    assert_eq!(docs[1].payload, json!("just a string"), "a string scalar round-trips");
    assert_eq!(docs[1].extra, None, "SQL NULL is None");
}

#[tokio::test]
async fn raw_value_json_flows_through_the_query_path() {
    let pool_ = pool();
    migrate::create_table::<Doc, _>(&pool_).await.unwrap();
    pool_
        .execute(
            "INSERT INTO docs (payload, extra) VALUES (?, ?)",
            &[Value::Json(json!({ "n": 7 })), Value::Json(json!("extra"))],
        )
        .await
        .unwrap();

    let docs = QuerySet::<Doc>::new("docs")
        .filter_eq("payload", Value::Json(json!({ "n": 7 })))
        .unwrap()
        .all(&pool_)
        .await
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].payload, json!({ "n": 7 }));
    assert_eq!(docs[0].extra, Some(json!("extra")));
}

#[tokio::test]
async fn invalid_json_text_is_a_decode_error_not_a_silent_string() {
    let pool_ = pool();
    migrate::create_table::<Doc, _>(&pool_).await.unwrap();
    pool_.execute("INSERT INTO docs (payload, extra) VALUES ('{oops', NULL)", &[]).await.unwrap();

    let err = QuerySet::<Doc>::new("docs").all(&pool_).await.err().unwrap();
    assert!(matches!(&err, OrmError::QueryError(msg) if msg.contains("invalid JSON")), "{err}");
}
