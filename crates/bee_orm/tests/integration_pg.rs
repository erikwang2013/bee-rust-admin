// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "postgres")]
//! PostgreSQL integration tests against a real server. Each test is a no-op
//! (with a skip notice) unless `BEE_ORM_PG_DSN` is set; CI sets it against a
//! `postgres:16-alpine` service container.
mod common;

use std::time::Duration;

use bee_orm::pool::postgres::Pool;
use bee_orm::{Model, OrmError, Value, m2m, migrate};
use serde_json::json;

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_pg_users")]
struct PgUser {
    #[bee(pk, auto)]
    id: i64,
    name: String,
    age: Option<i32>,
    score: f64,
    active: bool,
    note: Option<String>,
    data: Vec<u8>,
}

fn alice() -> PgUser {
    PgUser {
        id: 0,
        name: "alice".into(),
        age: Some(30),
        score: 1.5,
        active: true,
        note: None,
        data: vec![0, 1, 2, 255],
    }
}

#[tokio::test]
async fn roundtrip() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_pg_users", &[]).await?;
    pool.execute(
        "CREATE TABLE it_pg_users (id BIGSERIAL PRIMARY KEY, name TEXT NOT NULL, age INTEGER, \
         score DOUBLE PRECISION NOT NULL, active BOOLEAN NOT NULL, note TEXT, data BYTEA)",
        &[],
    )
    .await?;

    // Model insert skips the `auto` pk, so the server assigns the key.
    assert_eq!(alice().insert(&pool).await?, 1);
    let found = PgUser::query().filter_eq("name", "alice")?.one(&pool).await?.unwrap();
    assert_ne!(found.id, 0);
    assert_eq!(found, PgUser { id: found.id, ..alice() });

    // Value fidelity, straight from the wire.
    let rows = pool
        .query(
            "SELECT name, age, score, active, note, data FROM it_pg_users WHERE id = ?",
            &[Value::Int(found.id)],
        )
        .await?;
    assert_eq!(rows[0]["name"], "alice");
    assert_eq!(rows[0]["age"], 30);
    assert_eq!(rows[0]["score"], 1.5);
    assert_eq!(rows[0]["active"], true);
    assert!(rows[0]["note"].is_null());
    assert_eq!(bee_orm::decode::<Vec<u8>>(&rows[0], "data")?, vec![0, 1, 2, 255]);

    // Model update is scoped to the pk.
    let mut edited = found.clone();
    edited.name = "alicia".into();
    edited.age = Some(31);
    edited.note = Some("hi".into());
    assert_eq!(edited.update(&pool).await?, 1);
    assert_eq!(PgUser::query().filter_eq("id", found.id)?.one(&pool).await?.unwrap(), edited);

    // QuerySet read with filter / order / limit.
    let bob = PgUser { id: 0, name: "bob".into(), age: Some(16), ..alice() };
    assert_eq!(bob.insert(&pool).await?, 1);
    let adults: Vec<String> = PgUser::query()
        .filter_gt("age", 18)?
        .order_by("id DESC")
        .limit(10)
        .all(&pool)
        .await?
        .into_iter()
        .map(|user| user.name)
        .collect();
    assert_eq!(adults, vec!["alicia"]);

    // Model delete, then QuerySet delete.
    assert_eq!(edited.delete(&pool).await?, 1);
    assert_eq!(PgUser::query().count(&pool).await?, 1);
    assert_eq!(PgUser::query().filter_eq("name", "bob")?.delete(&pool).await?, 1);
    assert_eq!(PgUser::query().count(&pool).await?, 0);
    Ok(())
}

/// Dropping a `CheckedConn` mid-transaction spawns the ROLLBACK task; the
/// rolled-back row must not survive. `max_size = 1` forces the next query onto
/// that same connection.
#[tokio::test]
async fn drop_rolls_back_the_open_transaction() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 1)?;
    pool.execute("DROP TABLE IF EXISTS it_pg_drop_tx", &[]).await?;
    pool.execute("CREATE TABLE it_pg_drop_tx (v INTEGER)", &[]).await?;

    {
        let conn = pool.get().await?;
        conn.begin().await?;
        conn.execute("INSERT INTO it_pg_drop_tx (v) VALUES (?)", &[Value::Int(1)]).await?;
        // No commit: Drop spawns the ROLLBACK on the current runtime.
    }
    // Give the spawned task a moment to finish before checking (§15.1).
    tokio::time::sleep(Duration::from_millis(100)).await;

    let rows = pool.query("SELECT COUNT(*) AS count FROM it_pg_drop_tx", &[]).await?;
    assert_eq!(rows[0]["count"], 0);
    Ok(())
}

/// Holding the only connection makes the next `get()` wait out the real 30 s
/// timeout, which must surface as a `pool exhausted` connection error. No
/// wall-clock assertion: the mapping is the contract, the duration is not.
#[tokio::test]
async fn pool_exhaustion_reports_pool_exhausted() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 1)?;
    let _held = pool.get().await?;

    let err = match pool.get().await {
        Ok(_) => panic!("the second get() must not hand out a connection"),
        Err(err) => err,
    };
    assert!(
        matches!(&err, OrmError::ConnectionError(message) if message.contains("pool exhausted")),
        "unexpected error: {err}"
    );
    Ok(())
}

/// `prepare_cached` reuses the prepared statement behind a repeated SQL text;
/// the second call must behave exactly like the first (behavior only — no
/// assertion on driver internals).
#[tokio::test]
async fn cached_statements_stay_equivalent_on_reuse() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 1)?;

    let first = pool.query("SELECT 1 AS n", &[]).await?;
    let second = pool.query("SELECT 1 AS n", &[]).await?;
    assert_eq!(first, second);

    let first = pool.query("SELECT ?::int4 AS n", &[Value::Int(7)]).await?;
    let second = pool.query("SELECT ?::int4 AS n", &[Value::Int(7)]).await?;
    assert_eq!(first, second);
    assert_eq!(second[0]["n"], 7);
    Ok(())
}

/// `sslmode=require` against a server without TLS must fail the first `get()`
/// rather than silently connect in the clear.
#[cfg(feature = "postgres-tls")]
#[tokio::test]
async fn tls_require_against_a_non_tls_server_errors() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let separator = if dsn.contains('?') { '&' } else { '?' };
    let pool = Pool::connect_tls(&format!("{dsn}{separator}sslmode=require"), 1)?;

    let err = match pool.get().await {
        Ok(_) => panic!("a TLS handshake against a non-TLS server must fail"),
        Err(err) => err,
    };
    assert!(matches!(err, OrmError::ConnectionError(_)), "unexpected error: {err}");
    Ok(())
}

// ------------------------------------------- round 4: migrations + real FK

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_pg_teams")]
struct PgTeam {
    #[bee(pk, auto)]
    id: i64,
    name: String,
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_pg_members")]
struct PgMember {
    #[bee(pk, auto)]
    id: i64,
    #[bee(fk = PgTeam)]
    team_id: i64,
    name: String,
}

/// §39: the generated DDL is really valid postgres — identity pk, an inline
/// `REFERENCES` pair (parents first) and `sync` idempotency on a live server.
#[tokio::test]
async fn migrations_create_the_fk_pair_and_stay_idempotent() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_pg_members", &[]).await?;
    pool.execute("DROP TABLE IF EXISTS it_pg_teams", &[]).await?;

    // §36/§39: the inline `REFERENCES` is enforced here — creating the child
    // while the parent is absent fails loudly.
    assert!(
        migrate::create_table::<PgMember, _>(&pool).await.is_err(),
        "postgres must reject a child table whose parent does not exist"
    );

    // Parents before children: the `REFERENCES` target must exist.
    migrate::create_table::<PgTeam, _>(&pool).await?;
    migrate::create_table::<PgMember, _>(&pool).await?;
    assert_eq!(migrate::sync::<PgTeam, _>(&pool).await?, 0, "columns created above");
    assert_eq!(migrate::sync::<PgMember, _>(&pool).await?, 0, "idempotent on a live table");

    // The auto pk is `GENERATED BY DEFAULT AS IDENTITY`: explicit ids keep
    // working, matching sqlite/mysql semantics (§40).
    pool.execute("INSERT INTO it_pg_teams (id, name) VALUES (100, 'explicit')", &[]).await?;
    let team = PgTeam::query().filter_eq("id", 100)?.one(&pool).await?.unwrap();
    assert_eq!(team.name, "explicit");

    // The fk column is a plain writable column, and postgres enforces it.
    let member = PgMember { id: 0, team_id: team.id, name: "root".into() };
    assert_eq!(member.insert(&pool).await?, 1);
    let found = PgMember::query().filter_eq("team_id", team.id)?.one(&pool).await?.unwrap();
    assert_eq!(found.name, "root");
    let dangling = PgMember { id: 0, team_id: 999_999, name: "ghost".into() };
    assert!(dangling.insert(&pool).await.is_err(), "a dangling FK must be rejected");
    Ok(())
}

// ------------------------------------------ round 5: jsonb + m2m end-to-end

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_pg_docs")]
struct PgDoc {
    #[bee(pk, auto)]
    id: i64,
    payload: serde_json::Value,
    extra: Option<serde_json::Value>,
}

/// §45/§47: `serde_json::Value` renders `JSONB` and round-trips object,
/// string scalar and JSON `null`. The last one rides the `TypeId` read seam:
/// on pg the pool re-serializes a non-NULL `Json::Null` cell to the text
/// `"null"`, so `Option<serde_json::Value>` can tell it from SQL `NULL`.
#[tokio::test]
async fn jsonb_columns_round_trip_object_scalar_and_null() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_pg_docs", &[]).await?;
    migrate::create_table::<PgDoc, _>(&pool).await?;

    let rows = pool
        .query(
            "SELECT data_type AS ty FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'it_pg_docs' \
             AND column_name = 'payload'",
            &[],
        )
        .await?;
    assert_eq!(rows[0]["ty"], "jsonb", "serde_json::Value renders JSONB on postgres");

    let object = json!({ "k": [1, true, "x"] });
    PgDoc { id: 0, payload: object.clone(), extra: Some(json!(null)) }.insert(&pool).await?;
    PgDoc { id: 0, payload: json!("just a string"), extra: None }.insert(&pool).await?;

    let docs = PgDoc::query().order_by("id").all(&pool).await?;
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].payload, object, "object round-trips");
    assert_eq!(docs[0].extra, Some(json!(null)), "a stored JSON null is Some(null)");
    assert_eq!(docs[1].payload, json!("just a string"), "a string scalar round-trips");
    assert_eq!(docs[1].extra, None, "SQL NULL is None");

    // Raw-cell side of §45's pinned null semantics: the SQL NULL cell is the
    // only `Json::Null` cell — a non-Option target gets `Json::Null` for it
    // while the Option target gets `None` (the stored-null side is pinned by
    // doc 0 above).
    let rows = pool.query("SELECT extra FROM it_pg_docs WHERE extra IS NULL", &[]).await?;
    assert_eq!(bee_orm::decode::<serde_json::Value>(&rows[0], "extra")?, json!(null));
    assert_eq!(bee_orm::decode::<Option<serde_json::Value>>(&rows[0], "extra")?, None);
    Ok(())
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_pg_tags")]
struct PgTag {
    #[bee(pk, auto)]
    id: i64,
    label: String,
    #[bee(soft_delete)]
    deleted: bool,
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_pg_posts")]
#[bee(m2m(PgTag))]
struct PgPost {
    #[bee(pk, auto)]
    id: i64,
    title: String,
}

/// §43/§47: the whole m2m chain against a live postgres. The join table is
/// the ident-lowercase default `pgpost_pgtag` (§43 B1 — `#[bee(table = …)]`
/// renames do not move it), its generated DDL must pass postgres validation,
/// and the two-step read applies the target's soft filter while
/// `related_ids` stays raw.
#[tokio::test]
async fn m2m_end_to_end_against_the_real_server() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    // Reverse of creation: the join table references both sides.
    pool.execute("DROP TABLE IF EXISTS pgpost_pgtag", &[]).await?;
    pool.execute("DROP TABLE IF EXISTS it_pg_posts", &[]).await?;
    pool.execute("DROP TABLE IF EXISTS it_pg_tags", &[]).await?;
    migrate::create_table::<PgTag, _>(&pool).await?;
    migrate::create_table::<PgPost, _>(&pool).await?;

    let post = PgPost { id: 0, title: "hello".into() };
    post.insert(&pool).await?;
    let post = PgPost::query().filter_eq("title", "hello")?.one(&pool).await?.unwrap();
    let rust = PgTag { id: 0, label: "rust".into(), deleted: false };
    rust.insert(&pool).await?;
    let rust = PgTag::query().filter_eq("label", "rust")?.one(&pool).await?.unwrap();
    let db = PgTag { id: 0, label: "db".into(), deleted: false };
    db.insert(&pool).await?;
    let db = PgTag::query().filter_eq("label", "db")?.one(&pool).await?.unwrap();

    assert_eq!(m2m::attach(&pool, &post, &rust).await?, 1);
    assert_eq!(m2m::attach(&pool, &post, &db).await?, 1);
    // A duplicate pair is the join table's composite primary key, loud (§43).
    assert!(
        matches!(m2m::attach(&pool, &post, &rust).await, Err(OrmError::QueryError(_))),
        "duplicate attach must be a QueryError"
    );

    assert_eq!(m2m::related::<PgPost, PgTag, _>(&pool, &post).await?.len(), 2);
    // Duplicate local inputs keep aligned, undeduped groups.
    let groups = m2m::related_for::<PgPost, PgTag, _>(&pool, &[post.clone(), post.clone()]).await?;
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].len(), 2);
    assert_eq!(groups[1].len(), 2);

    // The two-step read hides the soft-deleted target; the raw id list and
    // the join rows stay (§43 pinned).
    db.delete(&pool).await?;
    assert_eq!(m2m::related::<PgPost, PgTag, _>(&pool, &post).await?.len(), 1);
    let groups = m2m::related_for::<PgPost, PgTag, _>(&pool, std::slice::from_ref(&post)).await?;
    assert_eq!(groups[0].len(), 1);
    assert_eq!(m2m::related_ids::<PgPost, PgTag, _>(&pool, &post).await?.len(), 2);

    assert_eq!(m2m::detach(&pool, &post, &rust).await?, 1);
    assert_eq!(m2m::detach(&pool, &post, &rust).await?, 0, "detach is idempotent");
    Ok(())
}

// -------------------------------- round 6: §56 chrono + decimal (§59#1-3 pg)

#[cfg(feature = "chrono")]
#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_pg_times")]
struct PgTime {
    #[bee(pk, auto)]
    id: i64,
    t: chrono::DateTime<chrono::Utc>,
    d: chrono::NaiveDate,
}

/// §59#1/#2: TIMESTAMPTZ ↔ `DateTime<Utc>` goes through the driver types (no
/// text round-trip) and normalizes to the same microsecond instant no matter
/// which offset spelling it arrived from; DATE keeps no time component.
#[cfg(feature = "chrono")]
#[tokio::test]
async fn timestamptz_normalizes_to_utc_and_date_has_no_time() -> Result<(), OrmError> {
    use chrono::{DateTime, NaiveDate, Utc};

    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_pg_times", &[]).await?;
    migrate::create_table::<PgTime, _>(&pool).await?;

    // The source instant is spelled with a +05:30 offset — the round trip must
    // keep the instant, not the spelling.
    let source = DateTime::parse_from_rfc3339("2026-10-06T17:04:56.123456+05:30").unwrap();
    let utc = source.with_timezone(&Utc);
    let date = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
    PgTime { id: 0, t: utc, d: date }.insert(&pool).await?;
    // A raw-SQL row with the same non-UTC spelling, server-side.
    pool.execute(
        "INSERT INTO it_pg_times (t, d) \
         VALUES (TIMESTAMPTZ '2026-10-06 17:04:56.123456+05:30', DATE '2026-10-06')",
        &[],
    )
    .await?;

    let rows = pool.query("SELECT t, d FROM it_pg_times ORDER BY id", &[]).await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["t"], json!("2026-10-06T11:34:56.123456Z"), "bind side is the UTC instant");
    assert_eq!(rows[1]["t"], rows[0]["t"], "the raw +05:30 row is the same instant");
    assert_eq!(bee_orm::decode::<DateTime<Utc>>(&rows[0], "t")?, utc);
    assert_eq!(bee_orm::decode::<DateTime<Utc>>(&rows[1], "t")?, utc);
    for row in &rows {
        assert_eq!(row["d"], json!("2026-10-06"), "date keeps no time component");
        assert_eq!(bee_orm::decode::<NaiveDate>(row, "d")?, date);
    }
    // The derived Model read path agrees (insert skipped the auto pk).
    let found = PgTime::query().order_by("id").one(&pool).await?.unwrap();
    assert_eq!(found, PgTime { id: found.id, t: utc, d: date });
    assert_ne!(found.id, 0);
    Ok(())
}

#[cfg(feature = "rust_decimal")]
#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_pg_nums")]
struct PgNum {
    #[bee(pk, auto)]
    id: i64,
    n: rust_decimal::Decimal,
}

/// §59#3 (pg, native): an in-range `Decimal` round-trips numerically equal and
/// keeps its scale text (`numeric` has no typmod); a value past the 96-bit
/// mantissa ceiling fails `FromSql` and reads as the documented `Json::Null`
/// cell — no decode-time error on the pg side (that asymmetry is mysql's).
#[cfg(feature = "rust_decimal")]
#[tokio::test]
async fn numeric_round_trips_and_the_precision_ceiling_is_a_null_cell() -> Result<(), OrmError> {
    use rust_decimal::Decimal;

    let Some(dsn) = common::dsn("BEE_ORM_PG_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_pg_nums", &[]).await?;
    migrate::create_table::<PgNum, _>(&pool).await?;

    let decimal: Decimal = "1.50".parse().unwrap();
    PgNum { id: 0, n: decimal }.insert(&pool).await?;
    let found = PgNum::query().order_by("id").one(&pool).await?.unwrap();
    assert_eq!(found.n, decimal, "in-range round trip is numerically equal");
    let rows = pool.query("SELECT n FROM it_pg_nums ORDER BY id", &[]).await?;
    assert_eq!(rows[0]["n"], json!("1.50"), "pg numeric keeps the scale text");

    // 30 nines: one significant digit past the 96-bit mantissa (<= 29 digits).
    pool.execute("INSERT INTO it_pg_nums (n) VALUES (999999999999999999999999999999)", &[]).await?;
    let rows = pool.query("SELECT n FROM it_pg_nums ORDER BY id", &[]).await?;
    assert_eq!(rows[0]["n"], json!("1.50"));
    assert_eq!(rows[1]["n"], json!(null), "beyond the ceiling the cell is Json::Null");
    assert!(bee_orm::decode::<Decimal>(&rows[1], "n").is_err(), "…and decoding it errors");
    Ok(())
}
