// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Unit tests for [`crate::m2m`] and the join-table DDL of
//! [`crate::migrate`]: SQL shapes against the recording mock. Real-backend
//! behavior lives in `tests/orm_m2m_json.rs`.

use serde_json::json;

use super::{Author, Mock, Reader, Setting, row_of};
use crate::{Dialect, MigrateOptions, OrmError, Value, m2m, migrate};

#[tokio::test]
async fn join_table_ddl_is_the_pinned_shape_per_dialect() {
    let sqlite = Mock::with_dialect(Dialect::Sqlite);
    migrate::create_table::<Reader, _>(&sqlite).await.unwrap();
    assert_eq!(
        sqlite.calls()[1].0,
        "CREATE TABLE IF NOT EXISTS reader_author (reader_id INTEGER NOT NULL REFERENCES readers (id), \
         author_id INTEGER NOT NULL REFERENCES authors (id), PRIMARY KEY (reader_id, author_id), \
         UNIQUE (author_id, reader_id))"
    );

    let pg = Mock::with_dialect(Dialect::Postgres);
    migrate::create_table::<Reader, _>(&pg).await.unwrap();
    // `BigInt` stays BIGINT on postgres; the inline reference is the default.
    assert!(
        pg.calls()[1].0.contains("reader_id BIGINT NOT NULL REFERENCES readers (id)"),
        "{}",
        pg.calls()[1].0
    );

    // mysql without the option: inline `REFERENCES` (parsed and ignored).
    let mysql = Mock::with_dialect(Dialect::Mysql);
    migrate::create_table::<Reader, _>(&mysql).await.unwrap();
    assert!(mysql.calls()[1].0.contains("reader_id BIGINT NOT NULL REFERENCES readers (id)"));
}

#[tokio::test]
async fn the_table_level_option_is_mysql_only_and_moves_fks_to_the_body() {
    let pg = Mock::with_dialect(Dialect::Postgres);
    migrate::create_table_with::<Reader, _>(&pg, MigrateOptions { table_level_fk: true })
        .await
        .unwrap();
    let ddl = &pg.calls()[0].0;
    assert!(ddl.contains("author_id BIGINT NOT NULL REFERENCES authors (id)"), "{ddl}");
    assert!(!ddl.contains("FOREIGN KEY ("), "pg keeps the inline form: {ddl}");
    assert!(pg.calls()[1].0.contains("REFERENCES readers (id)"), "join table too");

    let mysql = Mock::with_dialect(Dialect::Mysql);
    migrate::create_table_with::<Reader, _>(&mysql, MigrateOptions { table_level_fk: true })
        .await
        .unwrap();
    let ddl = &mysql.calls()[0].0;
    assert!(!ddl.contains("author_id BIGINT NOT NULL REFERENCES"), "{ddl}");
    assert!(
        ddl.contains("author_id BIGINT NOT NULL, FOREIGN KEY (author_id) REFERENCES authors (id)")
    );
    assert_eq!(
        mysql.calls()[1].0,
        "CREATE TABLE IF NOT EXISTS reader_author (reader_id BIGINT NOT NULL, author_id BIGINT NOT NULL, \
         FOREIGN KEY (reader_id) REFERENCES readers (id), FOREIGN KEY (author_id) REFERENCES authors (id), \
         PRIMARY KEY (reader_id, author_id), UNIQUE (author_id, reader_id))"
    );
}

#[tokio::test]
async fn mysql_add_path_splits_the_column_from_the_named_constraint() {
    // Empty preset rows: every column reads as missing and no named
    // constraint is found.
    let mock = Mock::with_dialect(Dialect::Mysql);
    migrate::add_missing_columns_with::<Reader, _>(&mock, MigrateOptions { table_level_fk: true })
        .await
        .unwrap();
    let calls = mock.calls();
    let sql: Vec<&str> = calls.iter().map(|(sql, _)| sql.as_str()).collect();
    assert!(sql.contains(&"ALTER TABLE readers ADD COLUMN author_id BIGINT NOT NULL"), "{sql:?}");
    assert!(
        sql.contains(
            &"ALTER TABLE readers ADD CONSTRAINT readers_author_id_fk FOREIGN KEY (author_id) \
              REFERENCES authors (id)"
        ),
        "{sql:?}"
    );
    // The primary key has no reference: no constraint is emitted for it.
    assert!(!sql.iter().any(|sql| sql.contains("readers_id_fk")), "{sql:?}");
    // The constraint lookup binds table and name.
    let lookup =
        calls.iter().find(|(sql, _)| sql.contains("table_constraints")).expect("constraint lookup");
    assert_eq!(
        lookup.1,
        vec![Value::Text("readers".into()), Value::Text("readers_author_id_fk".into())]
    );
}

#[tokio::test]
async fn related_ids_reads_the_join_table_and_maps_cells_back() {
    let mock = Mock::with_rows(vec![row_of(&[("author_id", json!(7))])]);
    let ids = m2m::related_ids::<Reader, Author, _>(&mock, &Reader { id: 3 }).await.unwrap();
    assert_eq!(ids, vec![Value::Int(7)]);
    assert_eq!(
        mock.calls()[0],
        (
            "SELECT author_id FROM reader_author WHERE reader_id = ?".to_string(),
            vec![Value::Int(3)]
        )
    );
}

#[tokio::test]
async fn attach_and_detach_bind_both_primary_keys() {
    let mock = Mock::default();
    assert_eq!(m2m::attach(&mock, &Reader { id: 3 }, &Author { id: 7 }).await.unwrap(), 1);
    assert_eq!(mock.calls()[0].0, "INSERT INTO reader_author (reader_id, author_id) VALUES (?, ?)");
    assert_eq!(mock.calls()[0].1, vec![Value::Int(3), Value::Int(7)]);

    assert_eq!(m2m::detach(&mock, &Reader { id: 3 }, &Author { id: 7 }).await.unwrap(), 1);
    assert_eq!(
        mock.calls()[1].0,
        "DELETE FROM reader_author WHERE reader_id = ? AND author_id = ?"
    );
    assert_eq!(mock.calls()[1].1, vec![Value::Int(3), Value::Int(7)]);
}

#[tokio::test]
async fn related_for_groups_are_index_aligned_and_unknown_locals_empty() {
    // One row serves both queries of the two-step read: the pair query reads
    // local_key/foreign_key, the row query the pk.
    let row = row_of(&[("local_key", json!(1)), ("foreign_key", json!(2)), ("id", json!(2))]);
    let mock = Mock::with_rows(vec![row]);
    let groups = m2m::related_for::<Reader, Author, _>(
        &mock,
        &[Reader { id: 1 }, Reader { id: 9 }, Reader { id: 1 }],
    )
    .await
    .unwrap();
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0][0].id, 2);
    assert!(groups[1].is_empty());
    assert_eq!(groups[2][0].id, 2);
}

#[tokio::test]
async fn no_locals_is_no_query() {
    let mock = Mock::with_dialect(Dialect::Sqlite);
    assert!(m2m::related_for::<Reader, Author, _>(&mock, &[]).await.unwrap().is_empty());
    assert!(mock.calls().is_empty());
}

#[tokio::test]
async fn a_missing_relation_is_an_error_naming_both_models() {
    let mock = Mock::default();
    assert!(m2m::m2m_def::<Reader, Author>().is_some());
    assert!(m2m::m2m_def::<Author, Reader>().is_none());

    // Author -> Reader: no forward def, but Reader declares the reverse, so the
    // hint names the target by its *ident* ("Author"), not its table ("authors").
    let err = m2m::related::<Author, Reader, _>(&mock, &Author { id: 1 }).await.err().unwrap();
    assert!(
        matches!(&err, OrmError::QueryError(msg)
            if msg.contains("authors") && msg.contains("readers") && msg.contains("#[bee(m2m(")),
        "{err}"
    );
    let OrmError::QueryError(msg) = &err else { panic!("expected QueryError, got {err}") };
    assert!(msg.contains("declares the reverse"), "{msg}");
    assert!(msg.contains("m2m(Author)"), "{msg}");
    // The old hint spliced the *table* name in (`#[bee(m2m(readers))]`), which
    // is not a type and does not compile — Author's table is `authors`.
    assert!(!msg.contains("m2m(readers"), "{msg}");

    // Neither direction declares the relation: no ident is available, so the
    // hint falls back to the `<Target>` placeholder and still splices no table
    // name into the attribute.
    let err = m2m::related::<Reader, Setting, _>(&mock, &Reader { id: 1 }).await.err().unwrap();
    let OrmError::QueryError(msg) = &err else { panic!("expected QueryError, got {err}") };
    assert!(msg.contains("m2m(<Target>)"), "{msg}");
    assert!(!msg.contains("m2m(readers") && !msg.contains("m2m(settings"), "{msg}");
}
