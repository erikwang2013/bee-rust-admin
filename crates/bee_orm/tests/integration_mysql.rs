// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "mysql")]
//! MySQL integration tests against a real server. Each test is a no-op (with a
//! skip notice) unless `BEE_ORM_MYSQL_DSN` is set; CI sets it against a
//! `mysql:8` service container.
mod common;

use bee_orm::pool::mysql::Pool;
use bee_orm::{Model, OrmError, Value, migrate};
use serde_json::json;

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_mysql_users")]
struct MyUser {
    #[bee(pk, auto)]
    id: i64,
    name: String,
    age: Option<i32>,
    score: f64,
    active: bool,
    note: Option<String>,
    /// `BLOB`, written as bytes and read back as UTF-8 text (the mysql read
    /// side is deliberately lossy there), so the field is a `String`.
    data: String,
}

fn alice() -> MyUser {
    MyUser {
        id: 0,
        name: "alice".into(),
        age: Some(30),
        score: 1.5,
        active: true,
        note: None,
        data: "hi".into(),
    }
}

#[tokio::test]
async fn roundtrip() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_mysql_users", &[]).await?;
    pool.execute(
        "CREATE TABLE it_mysql_users (id BIGINT AUTO_INCREMENT PRIMARY KEY, \
         name VARCHAR(255) NOT NULL, age INT, score DOUBLE NOT NULL, \
         active TINYINT(1) NOT NULL, note TEXT, data BLOB)",
        &[],
    )
    .await?;

    // Model insert skips the `auto` pk, so the server assigns the key.
    assert_eq!(alice().insert(&pool).await?, 1);
    let found = MyUser::query().filter_eq("name", "alice")?.one(&pool).await?.unwrap();
    assert_ne!(found.id, 0);
    assert_eq!(found, MyUser { id: found.id, ..alice() });

    // Value fidelity, straight from the wire. `active` arrives as `1` and only
    // decodes because `decode` normalises 0/1 to a boolean.
    let rows = pool
        .query(
            "SELECT name, age, score, active, note, data FROM it_mysql_users WHERE id = ?",
            &[Value::Int(found.id)],
        )
        .await?;
    assert_eq!(rows[0]["name"], "alice");
    assert_eq!(rows[0]["age"], 30);
    assert_eq!(rows[0]["score"], 1.5);
    assert_eq!(rows[0]["active"], 1);
    assert!(bee_orm::decode::<bool>(&rows[0], "active")?);
    assert!(rows[0]["note"].is_null());
    // A BLOB comes back as text, not as a JSON array of byte numbers.
    assert_eq!(rows[0]["data"], "hi");

    // Model update is scoped to the pk.
    let mut edited = found.clone();
    edited.name = "alicia".into();
    edited.age = Some(31);
    edited.note = Some("hi".into());
    assert_eq!(edited.update(&pool).await?, 1);
    assert_eq!(MyUser::query().filter_eq("id", found.id)?.one(&pool).await?.unwrap(), edited);

    // QuerySet read with filter / order / limit.
    let bob = MyUser { id: 0, name: "bob".into(), age: Some(16), ..alice() };
    assert_eq!(bob.insert(&pool).await?, 1);
    let adults: Vec<String> = MyUser::query()
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
    assert_eq!(MyUser::query().count(&pool).await?, 1);
    assert_eq!(MyUser::query().filter_eq("name", "bob")?.delete(&pool).await?, 1);
    assert_eq!(MyUser::query().count(&pool).await?, 0);
    Ok(())
}

/// A transaction left open when the `CheckedConn` is dropped is rolled back by
/// the pool's check-out reset, so the row must not survive. `max_size = 1`
/// forces the next query onto that same connection.
#[tokio::test]
async fn checkout_reset_discards_the_open_transaction() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 1)?;
    pool.execute("DROP TABLE IF EXISTS it_mysql_drop_tx", &[]).await?;
    pool.execute("CREATE TABLE it_mysql_drop_tx (v INT)", &[]).await?;

    {
        let mut conn = pool.get().await?;
        conn.begin().await?;
        conn.execute("INSERT INTO it_mysql_drop_tx (v) VALUES (?)", &[Value::Int(1)]).await?;
        // No commit: dropping returns the connection, and the pool resets it
        // on the next check-out.
    }

    let rows = pool.query("SELECT COUNT(*) AS count FROM it_mysql_drop_tx", &[]).await?;
    assert_eq!(rows[0]["count"], 0);
    Ok(())
}

/// Holding the only connection makes the next `get()` wait out the real 30 s
/// timeout, which must surface as a `pool exhausted` connection error. No
/// wall-clock assertion: the mapping is the contract, the duration is not.
#[tokio::test]
async fn pool_exhaustion_reports_pool_exhausted() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(()) };
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

// ------------------------------------------- round 4: migrations + real FK

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_mysql_teams")]
struct MyTeam {
    #[bee(pk, auto)]
    id: i64,
    name: String,
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_mysql_members")]
struct MyMember {
    #[bee(pk, auto)]
    id: i64,
    #[bee(fk = MyTeam)]
    team_id: i64,
    name: String,
}

/// §39: the generated DDL is really valid MySQL — auto pk, the inline
/// `REFERENCES` pair and `sync` idempotency on a live server.
///
/// MySQL parses but *ignores* `REFERENCES` inside a column clause (measured on
/// 8.0.46: no constraint is created, no error even for a missing target). The
/// §35 amendment / §40 ruling keep the inline clause as portable *intent* with
/// enforcement parity deferred to round 5+, so the dangling create below
/// **succeeding** is the pinned behavior and no insert is asserted to fail.
#[tokio::test]
async fn migrations_create_the_fk_pair_and_stay_idempotent() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_mysql_members", &[]).await?;
    pool.execute("DROP TABLE IF EXISTS it_mysql_teams", &[]).await?;

    // §36/§39: the child is created happily while the parent is still absent;
    // the `?` is the assertion (a failure to create would fail the test).
    migrate::create_table::<MyMember, _>(&pool).await?;
    migrate::create_table::<MyTeam, _>(&pool).await?;
    assert_eq!(migrate::sync::<MyTeam, _>(&pool).await?, 0, "columns created above");
    assert_eq!(migrate::sync::<MyMember, _>(&pool).await?, 0, "idempotent on a live table");

    // `AUTO_INCREMENT` accepts explicit ids, matching sqlite/postgres (§40).
    pool.execute("INSERT INTO it_mysql_teams (id, name) VALUES (100, 'explicit')", &[]).await?;
    let team = MyTeam::query().filter_eq("id", 100)?.one(&pool).await?.unwrap();
    assert_eq!(team.name, "explicit");

    let member = MyMember { id: 0, team_id: team.id, name: "root".into() };
    assert_eq!(member.insert(&pool).await?, 1);
    let found = MyMember::query().filter_eq("team_id", team.id)?.one(&pool).await?.unwrap();
    assert_eq!(found.name, "root");
    Ok(())
}

// ---------------------------------------------- round 5: json columns (§45)

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_mysql_docs")]
struct MyDoc {
    #[bee(pk, auto)]
    id: i64,
    payload: serde_json::Value,
    extra: Option<serde_json::Value>,
}

/// §45/§47: `serde_json::Value` renders the real mysql `JSON` type (not
/// TEXT) and round-trips object, string scalar and JSON `null`. The write
/// side binds the serialized bytes; the read side gets them back as a text
/// cell that the strict `TypeId` seam parses — SQL `NULL` stays `None`.
#[tokio::test]
async fn json_columns_round_trip_object_scalar_and_null() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_mysql_docs", &[]).await?;
    migrate::create_table::<MyDoc, _>(&pool).await?;

    let rows = pool
        .query(
            "SELECT data_type AS ty FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = 'it_mysql_docs' \
             AND column_name = 'payload'",
            &[],
        )
        .await?;
    assert_eq!(rows[0]["ty"], "json", "serde_json::Value renders JSON on mysql");

    let object = json!({ "k": [1, true, "x"] });
    MyDoc { id: 0, payload: object.clone(), extra: Some(json!(null)) }.insert(&pool).await?;
    MyDoc { id: 0, payload: json!("just a string"), extra: None }.insert(&pool).await?;

    let docs = MyDoc::query().order_by("id").all(&pool).await?;
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].payload, object, "object round-trips");
    assert_eq!(docs[0].extra, Some(json!(null)), "a stored JSON null is Some(null)");
    assert_eq!(docs[1].payload, json!("just a string"), "a string scalar round-trips");
    assert_eq!(docs[1].extra, None, "SQL NULL is None");
    Ok(())
}

// --------------------------------- round 6: §56 chrono + decimal (§59#5, #2, #3)

#[cfg(feature = "chrono")]
#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_mysql_times")]
struct MyTime {
    #[bee(pk, auto)]
    id: i64,
    dt: chrono::NaiveDateTime,
    ts: chrono::DateTime<chrono::Utc>,
    d: chrono::NaiveDate,
}

/// §59#5/#2: `datetime(6)` / `timestamp(6)` keep microseconds, and the read
/// side dispatches on the *real* column type — `datetime` is the naive cell,
/// `timestamp` the UTC one with a `Z`, `date` has no time. The TIMESTAMP ↔
/// `DateTime<Utc>` mapping assumes the session time zone is UTC: assert that
/// premise rather than assume it (both measured environments are UTC).
#[cfg(feature = "chrono")]
#[tokio::test]
async fn datetime_timestamp_micros_and_column_type_dispatch() -> Result<(), OrmError> {
    use chrono::{DateTime, NaiveDate, Utc};

    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;

    let rows =
        pool.query("SELECT @@system_time_zone AS sys, @@session.time_zone AS sess", &[]).await?;
    assert_eq!(rows[0]["sys"], json!("UTC"), "system tz must be UTC for the Z-cell premise");
    assert_eq!(rows[0]["sess"], json!("SYSTEM"), "an unnamed session tz follows the system one");

    pool.execute("DROP TABLE IF EXISTS it_mysql_times", &[]).await?;
    migrate::create_table::<MyTime, _>(&pool).await?;
    let rows = pool
        .query(
            "SELECT column_name AS c, column_type AS t FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = 'it_mysql_times'",
            &[],
        )
        .await?;
    let mut types = std::collections::HashMap::new();
    for row in &rows {
        types.insert(row["c"].as_str().unwrap(), row["t"].as_str().unwrap());
    }
    assert_eq!(types["dt"], "datetime(6)");
    assert_eq!(types["ts"], "timestamp(6)");
    assert_eq!(types["d"], "date");

    let naive = NaiveDate::from_ymd_opt(2026, 10, 6)
        .unwrap()
        .and_hms_micro_opt(12, 34, 56, 123456)
        .unwrap();
    let utc = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);
    let date = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
    MyTime { id: 0, dt: naive, ts: utc, d: date }.insert(&pool).await?;
    // The same values through raw SQL, server-side.
    pool.execute(
        "INSERT INTO it_mysql_times (dt, ts, d) \
         VALUES ('2026-10-06 12:34:56.123456', '2026-10-06 12:34:56.123456', '2026-10-06')",
        &[],
    )
    .await?;

    let rows = pool.query("SELECT dt, ts, d FROM it_mysql_times ORDER BY id", &[]).await?;
    assert_eq!(rows.len(), 2);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row["dt"], json!("2026-10-06T12:34:56.123456"), "datetime row {i} keeps micros");
        assert_eq!(
            row["ts"],
            json!("2026-10-06T12:34:56.123456Z"),
            "timestamp row {i} reads as UTC"
        );
        assert_eq!(row["d"], json!("2026-10-06"), "date row {i} has no time component");
    }
    assert_eq!(bee_orm::decode::<chrono::NaiveDateTime>(&rows[0], "dt")?, naive);
    assert_eq!(bee_orm::decode::<DateTime<Utc>>(&rows[0], "ts")?, utc);
    assert_eq!(bee_orm::decode::<NaiveDate>(&rows[0], "d")?, date);
    let found = MyTime::query().order_by("id").one(&pool).await?.unwrap();
    assert_eq!(found, MyTime { id: found.id, dt: naive, ts: utc, d: date });
    Ok(())
}

#[cfg(feature = "rust_decimal")]
#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_mysql_nums")]
struct MyNum {
    #[bee(pk, auto)]
    id: i64,
    n: rust_decimal::Decimal,
}

/// §59#3 (mysql, text): a `Decimal` binds as its decimal string; an in-range
/// value round-trips numerically equal, while a past-ceiling value stays a
/// text cell and only fails at decode (the documented asymmetry — pg turns
/// the same row into a `Json::Null` cell).
#[cfg(feature = "rust_decimal")]
#[tokio::test]
async fn decimal_text_path_round_trips_in_range_and_errors_at_decode_beyond() -> Result<(), OrmError>
{
    use rust_decimal::Decimal;

    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_mysql_nums", &[]).await?;
    migrate::create_table::<MyNum, _>(&pool).await?;

    // Through the generated DDL (decimal(65,30)): an in-range value must read
    // back numerically equal.
    let decimal: Decimal = "1.50".parse().unwrap();
    MyNum { id: 0, n: decimal }.insert(&pool).await?;
    let found = MyNum::query().order_by("id").one(&pool).await?.unwrap();
    assert_eq!(found.n, decimal, "in-range round trip is numerically equal");

    // The text path itself, on a scale rust_decimal can represent.
    pool.execute("DROP TABLE IF EXISTS it_mysql_num_text", &[]).await?;
    pool.execute("CREATE TABLE it_mysql_num_text (n DECIMAL(28,10))", &[]).await?;
    pool.execute("INSERT INTO it_mysql_num_text (n) VALUES (?)", &[Value::from(decimal)]).await?;
    let rows = pool.query("SELECT n FROM it_mysql_num_text", &[]).await?;
    assert!(rows[0]["n"].is_string(), "decimal cells arrive as text: {}", rows[0]["n"]);
    assert_eq!(bee_orm::decode::<Decimal>(&rows[0], "n")?, decimal);

    // One past the rust_decimal ceiling (30 significant digits).
    pool.execute("DROP TABLE IF EXISTS it_mysql_num_big", &[]).await?;
    pool.execute("CREATE TABLE it_mysql_num_big (n DECIMAL(35,0))", &[]).await?;
    pool.execute("INSERT INTO it_mysql_num_big (n) VALUES ('123456789012345678901234567890')", &[])
        .await?;
    let rows = pool.query("SELECT n FROM it_mysql_num_big", &[]).await?;
    assert_eq!(rows[0]["n"], json!("123456789012345678901234567890"), "the big value stays text");
    assert!(bee_orm::decode::<Decimal>(&rows[0], "n").is_err(), "…and errors only at decode");
    Ok(())
}
