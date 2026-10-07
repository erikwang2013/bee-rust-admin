// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "sqlite")]
//! Round-4 acceptance on in-memory SQLite: migrations (`create_table` / `sync`,
//! §35), relations (`children*` / `belongs_to` / `filter_in`, §36) and the §38
//! interaction rows.
use bee_orm::pool::sqlite::Pool;
use bee_orm::{Model, OrmError, Value, decode, migrate, rel};

fn pool() -> Pool {
    // The documented single-connection clamp is what makes `:memory:` coherent:
    // every checkout sees the same database.
    Pool::connect(":memory:", 1).unwrap()
}

// ---------------------------------------------------------------- migrations

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "growth")]
struct Growth {
    #[bee(pk, auto)]
    id: i64,
    title: String,
    rating: Option<i32>,
    #[bee(auto_now_add)]
    created_at: i64,
    #[bee(auto_now)]
    updated_at: i64,
    #[bee(soft_delete)]
    deleted: bool,
}

#[tokio::test]
async fn create_table_pins_the_dialect_shapes_and_the_model_round_trips() {
    let pool = pool();
    migrate::create_table::<Growth, _>(&pool).await.unwrap();

    let rows = pool
        .query("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'growth'", &[])
        .await
        .unwrap();
    let ddl = decode::<String>(&rows[0], "sql").unwrap();
    assert!(ddl.contains("id INTEGER PRIMARY KEY AUTOINCREMENT"), "auto pk form: {ddl}");
    assert!(
        ddl.contains("rating INTEGER") && !ddl.contains("rating INTEGER NOT NULL"),
        "a plain Option field is nullable: {ddl}"
    );
    assert!(ddl.contains("created_at INTEGER NOT NULL DEFAULT 0"), "auto_now_add: {ddl}");
    assert!(ddl.contains("updated_at INTEGER NOT NULL DEFAULT 0"), "auto_now: {ddl}");
    assert!(ddl.contains("deleted INTEGER NOT NULL DEFAULT 0"), "soft delete: {ddl}");

    // The generated DDL and the model agree: insert + read back.
    let growth = Growth {
        id: 0,
        title: "first".into(),
        rating: Some(5),
        created_at: 0,
        updated_at: 0,
        deleted: false,
    };
    assert_eq!(growth.insert(&pool).await.unwrap(), 1);
    let found =
        Growth::query().filter_eq("title", "first").unwrap().one(&pool).await.unwrap().unwrap();
    assert_eq!(found.rating, Some(5));
    // One "now" per statement, so both timestamp columns carry it.
    assert!(found.created_at > 0);
    assert_eq!(found.updated_at, found.created_at);
    assert!(!found.deleted);
}

#[tokio::test]
async fn sync_adds_only_the_missing_column_then_a_second_run_is_zero() {
    let pool = pool();
    // A table from before `rating` existed — hand-made, so it really lacks it.
    pool.execute(
        "CREATE TABLE growth (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, \
         created_at INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT 0, \
         deleted INTEGER NOT NULL DEFAULT 0)",
        &[],
    )
    .await
    .unwrap();
    pool.execute("INSERT INTO growth (title) VALUES ('old row')", &[]).await.unwrap();

    assert_eq!(migrate::sync::<Growth, _>(&pool).await.unwrap(), 1, "exactly the missing `rating`");
    assert_eq!(migrate::sync::<Growth, _>(&pool).await.unwrap(), 0, "a second run adds nothing");

    // Reads work over the migrated table, and the pre-existing row is intact.
    let rows = Growth::query().all(&pool).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "old row");
    assert_eq!(rows[0].rating, None);

    let fresh = Growth {
        id: 0,
        title: "fresh".into(),
        rating: Some(4),
        created_at: 0,
        updated_at: 0,
        deleted: false,
    };
    assert_eq!(fresh.insert(&pool).await.unwrap(), 1);
    let stored =
        Growth::query().filter_eq("title", "fresh").unwrap().one(&pool).await.unwrap().unwrap();
    assert_eq!(stored.rating, Some(4));
    assert_eq!(Growth::query().count(&pool).await.unwrap(), 2, "never destructive");
}

// ----------------------------------------------------------------- relations

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "authors")]
struct Author {
    #[bee(pk, auto)]
    id: i64,
    name: String,
    #[bee(soft_delete)]
    deleted: bool,
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "posts")]
struct Post {
    #[bee(pk, auto)]
    id: i64,
    /// First-declared fk wins discovery; `editor_id` is the `_via` case.
    #[bee(fk = Author)]
    author_id: Option<i64>,
    #[bee(fk = Author)]
    editor_id: Option<i64>,
    title: String,
    #[bee(soft_delete)]
    deleted: bool,
}

fn titles(mut posts: Vec<Post>) -> Vec<String> {
    posts.sort_by(|a, b| a.title.cmp(&b.title));
    posts.into_iter().map(|post| post.title).collect()
}

/// Two live authors + one soft-deleted, and six posts: one deleted, one orphan
/// (NULL fk), one parented to the deleted author, one editor-only link.
async fn seed() -> (Pool, Author, Author, Author) {
    let pool = pool();
    migrate::create_table::<Author, _>(&pool).await.unwrap();
    migrate::create_table::<Post, _>(&pool).await.unwrap();

    let mut authors = Vec::new();
    for name in ["alice", "bob", "carol"] {
        Author { id: 0, name: name.into(), deleted: false }.insert(&pool).await.unwrap();
        authors.push(
            Author::query().filter_eq("name", name).unwrap().one(&pool).await.unwrap().unwrap(),
        );
    }
    let (alice, bob, carol) = (authors[0].clone(), authors[1].clone(), authors[2].clone());
    assert_eq!(carol.delete(&pool).await.unwrap(), 1, "carol is soft-deleted");

    for (title, author, editor) in [
        ("a1", Some(alice.id), Some(bob.id)),
        ("a2", Some(alice.id), None),
        ("b1", Some(bob.id), Some(alice.id)),
        ("b2", Some(bob.id), None),
        ("c1", Some(carol.id), None),
        ("orphan", None, None),
    ] {
        Post { id: 0, author_id: author, editor_id: editor, title: title.into(), deleted: false }
            .insert(&pool)
            .await
            .unwrap();
    }
    let a2 = Post::query().filter_eq("title", "a2").unwrap().one(&pool).await.unwrap().unwrap();
    assert_eq!(a2.delete(&pool).await.unwrap(), 1, "a2 is soft-deleted");

    (pool, alice, bob, carol)
}

#[tokio::test]
async fn relations_follow_the_fk_metadata_and_the_soft_filter() {
    let (pool, alice, bob, carol) = seed().await;

    assert_eq!(rel::fk_column_to::<Post, Author>(), Some("author_id"), "first-declared fk wins");
    assert_eq!(rel::fk_column_to::<Author, Post>(), None);

    // Children of a live parent: the child's own soft filter applies.
    assert_eq!(titles(rel::children::<Post, Author, _>(&pool, &alice).await.unwrap()), vec!["a1"]);
    assert_eq!(
        titles(rel::children::<Post, Author, _>(&pool, &bob).await.unwrap()),
        vec!["b1", "b2"]
    );

    // The query-builder variant is the escape hatch to soft-deleted children.
    let all = rel::children_query::<Post, Author>(&alice).unwrap().with_deleted();
    assert_eq!(titles(all.all(&pool).await.unwrap()), vec!["a1", "a2"]);

    // `_via` addresses the second fk to the same parent.
    let edited = rel::children_via::<Post, Author, _>(&pool, &alice, "editor_id").await.unwrap();
    assert_eq!(titles(edited), vec!["b1"]);

    // `children_for` is index-aligned (duplicates included); a soft-deleted
    // parent still yields its children — filtering is per-model.
    let ghost = Author { id: 9999, name: "ghost".into(), deleted: false };
    let parents = [alice.clone(), bob.clone(), alice.clone(), carol.clone(), ghost];
    let grouped: Vec<Vec<String>> = rel::children_for::<Post, Author, _>(&pool, &parents)
        .await
        .unwrap()
        .into_iter()
        .map(titles)
        .collect();
    assert_eq!(grouped.len(), parents.len());
    assert_eq!(grouped[0], vec!["a1"]);
    assert_eq!(grouped[1], vec!["b1", "b2"]);
    assert_eq!(grouped[2], grouped[0], "the duplicate parent is aligned");
    assert_eq!(grouped[3], vec!["c1"], "the deleted parent's children still load");
    assert!(grouped[4].is_empty(), "an unknown parent has no children");

    // No parents, no query.
    assert!(rel::children_for::<Post, Author, _>(&pool, &[]).await.unwrap().is_empty());

    // belongs_to: the parent's soft filter applies; a NULL fk never matches.
    let found = rel::belongs_to::<Author, _>(&pool, alice.id).await.unwrap().unwrap();
    assert_eq!(found.name, "alice");
    assert!(
        rel::belongs_to::<Author, _>(&pool, carol.id).await.unwrap().is_none(),
        "deleted parent"
    );
    assert!(rel::belongs_to::<Author, _>(&pool, Value::Null).await.unwrap().is_none(), "NULL fk");

    // No fk metadata is a loud error naming both models.
    let post = Post { id: 0, author_id: None, editor_id: None, title: "x".into(), deleted: false };
    let err = rel::children::<Author, Post, _>(&pool, &post).await.unwrap_err();
    assert!(
        matches!(&err, OrmError::QueryError(message) if message.contains("authors") && message.contains("posts")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn filter_in_binds_the_values_in_order_and_rejects_an_empty_list() {
    let (pool, _, _, _) = seed().await;

    let in_query = Post::query()
        .filter_in(
            "title",
            &[Value::Text("b2".into()), Value::Text("a1".into()), Value::Text("b1".into())],
        )
        .unwrap();
    assert_eq!(titles(in_query.all(&pool).await.unwrap()), vec!["a1", "b1", "b2"]);

    // The model's own soft filter still applies on top of the IN term.
    let deleted = Post::query().filter_in("title", &[Value::Text("a2".into())]).unwrap();
    assert!(deleted.all(&pool).await.unwrap().is_empty());

    // An empty list is a loud error, not a silent nothing-match.
    let err = match Post::query().filter_in("title", &[]) {
        Ok(_) => panic!("an empty value list must be rejected"),
        Err(err) => err,
    };
    assert!(
        matches!(&err, OrmError::QueryError(message) if message.contains("filter_in")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn sqlite_records_the_inline_references_and_the_bundled_build_enforces_it() {
    let pool = pool();
    // The clause is declarative: a dangling CREATE is accepted (mysql-like).
    migrate::create_table::<Post, _>(&pool).await.unwrap();
    let rows = pool
        .query("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'posts'", &[])
        .await
        .unwrap();
    let ddl = decode::<String>(&rows[0], "sql").unwrap();
    assert!(ddl.contains("REFERENCES authors (id)"), "the clause is recorded: {ddl}");

    // …but enforcement is ON in this build, despite the pool never running a
    // PRAGMA: `rusqlite`'s `bundled` feature compiles libsqlite3 with
    // `-DSQLITE_DEFAULT_FOREIGN_KEYS=1` (libsqlite3-sys build.rs, 0.30.1:123),
    // so `PRAGMA foreign_keys` is already 1 on every fresh connection and a
    // dangling row is rejected. Measured, not assumed — the §35 amendment's
    // "sqlite enforces nothing" wording is being corrected.
    let pragma = pool.query("PRAGMA foreign_keys", &[]).await.unwrap();
    assert_eq!(pragma[0]["foreign_keys"], 1, "bundled default, no pool-side PRAGMA");

    migrate::create_table::<Author, _>(&pool).await.unwrap();
    let post = Post {
        id: 0,
        author_id: Some(9999),
        editor_id: None,
        title: "dangling".into(),
        deleted: false,
    };
    let err = post.insert(&pool).await.unwrap_err();
    assert!(
        matches!(&err, OrmError::QueryError(message) if message.contains("FOREIGN KEY constraint failed")),
        "unexpected error: {err}"
    );
}

// ------------------------------------- ADD COLUMN × REFERENCES × DEFAULT
// §35 add-column guidance says a new NOT NULL column on a populated table needs
// a default, and the escape hatch is a `sql_type` carrying `DEFAULT`. This
// probes the fk-column corner of that advice: on a populated table neither
// shape can carry a `REFERENCES` clause.

#[derive(Model)]
#[bee(table = "fk_add_default")]
struct FkAddDefault {
    #[bee(pk, auto)]
    id: i64,
    #[bee(fk = Author, sql_type = "INTEGER DEFAULT 0")]
    author_id: i64,
}

#[derive(Model)]
#[bee(table = "fk_add_option")]
struct FkAddOption {
    #[bee(pk, auto)]
    id: i64,
    #[bee(fk = Author)]
    author_id: Option<i64>,
}

#[derive(Model)]
#[bee(table = "fk_add_notnull")]
struct FkAddNotNull {
    #[bee(pk, auto)]
    id: i64,
    #[bee(fk = Author)]
    author_id: i64,
}

#[tokio::test]
async fn sqlite_fk_add_column_restrictions_are_data_checks() {
    let pool = pool();
    migrate::create_table::<Author, _>(&pool).await.unwrap();
    // Child tables from "before" the fk column existed, each with a row.
    for table in ["fk_add_default", "fk_add_option", "fk_add_notnull"] {
        pool.execute(&format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY AUTOINCREMENT)"), &[])
            .await
            .unwrap();
        pool.execute(&format!("INSERT INTO {table} (id) VALUES (1)"), &[]).await.unwrap();
    }

    // `sql_type = "INTEGER DEFAULT 0"` renders
    // `author_id INTEGER DEFAULT 0 NOT NULL REFERENCES authors (id)`: with fk
    // enforcement on, sqlite refuses to add a REFERENCES column with a non-NULL
    // default to a populated table — the `sql_type`-with-DEFAULT escape hatch
    // cannot carry an fk.
    let err = migrate::add_missing_columns::<FkAddDefault, _>(&pool).await.unwrap_err();
    assert!(
        matches!(&err, OrmError::QueryError(message) if message.contains("Cannot add a REFERENCES column with non-NULL default value")),
        "unexpected error: {err}"
    );
    // Both ADD COLUMN restrictions measured here are data checks, not schema
    // checks: on an empty table sqlite accepts the very same ALTER.
    pool.execute("DELETE FROM fk_add_default", &[]).await.unwrap();
    assert_eq!(
        migrate::add_missing_columns::<FkAddDefault, _>(&pool).await.unwrap(),
        1,
        "empty table: even the non-NULL default goes through"
    );

    // Control: the same fk column as `Option<i64>` (NULL default) is accepted on
    // a populated table, and the ALTER path really did attach an enforced
    // reference.
    assert_eq!(migrate::add_missing_columns::<FkAddOption, _>(&pool).await.unwrap(), 1);
    let err = FkAddOption { id: 0, author_id: Some(9999) }.insert(&pool).await.unwrap_err();
    assert!(
        matches!(&err, OrmError::QueryError(message) if message.contains("FOREIGN KEY constraint failed")),
        "unexpected error: {err}"
    );

    // The sibling path — a non-Option fk with no default — is refused by the
    // NOT NULL rule on a populated table, and goes through once the table is
    // empty, matching the module caveat ("new NOT NULL columns without a default
    // only work on an empty table").
    let err = migrate::add_missing_columns::<FkAddNotNull, _>(&pool).await.unwrap_err();
    assert!(
        matches!(&err, OrmError::QueryError(message) if message.contains("Cannot add a NOT NULL column with default value NULL")),
        "unexpected error: {err}"
    );
    pool.execute("DELETE FROM fk_add_notnull", &[]).await.unwrap();
    assert_eq!(
        migrate::add_missing_columns::<FkAddNotNull, _>(&pool).await.unwrap(),
        1,
        "empty table: the NOT NULL fk column is added"
    );
}
