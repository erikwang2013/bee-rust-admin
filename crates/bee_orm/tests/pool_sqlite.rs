// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "sqlite")]
use bee_orm::Value;
use bee_orm::pool::sqlite::Pool;

/// Sequential use always reuses the same idle connection, so this does not
/// exercise the `:memory:` single-connection clamp (`Pool::connect` in
/// src/pool/sqlite.rs) — that is guaranteed by code review, not by this test.
#[tokio::test]
async fn sqlite_pool_roundtrip() {
    let pool = Pool::connect(":memory:", 4).unwrap();

    pool.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)", &[])
        .await
        .unwrap();

    let affected = pool
        .execute(
            "INSERT INTO users (name, age) VALUES (?, ?)",
            &[Value::from("alice"), Value::Int(30)],
        )
        .await
        .unwrap();
    assert_eq!(affected, 1);

    // Bind `30` as an integer: a text `"30"` would come back as the JSON
    // string `"30"` and fail the numeric assertion below.
    let rows = pool
        .query("SELECT name, age FROM users WHERE name = ?", &[Value::from("alice")])
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "alice");
    assert_eq!(rows[0]["age"], 30);
}

#[tokio::test]
async fn sqlite_checked_out_transaction() {
    let pool = Pool::connect(":memory:", 4).unwrap();
    let conn = pool.get().unwrap();

    conn.execute("CREATE TABLE t (v INTEGER)", &[]).unwrap();

    conn.begin().unwrap();
    conn.execute("INSERT INTO t (v) VALUES (?)", &[Value::Int(1)]).unwrap();
    conn.rollback().unwrap();
    assert!(conn.query("SELECT v FROM t", &[]).unwrap().is_empty());

    conn.begin().unwrap();
    conn.execute("INSERT INTO t (v) VALUES (?)", &[Value::Int(2)]).unwrap();
    conn.commit().unwrap();
    let rows = conn.query("SELECT v FROM t", &[]).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["v"], 2);

    drop(conn);
    // On a fresh check-out the committed row is still there and the
    // rolled-back one never was.
    let rows = pool.query("SELECT v FROM t", &[]).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["v"], 2);
}

/// A transaction dropped without `commit()` must not leak into the next
/// check-out: the drop hook rolls it back. `:memory:` clamps the pool to one
/// connection, so the next check-out is the same database — and round-1 code
/// (no drop hook) fails this test.
#[tokio::test]
async fn sqlite_dropped_transaction_is_rolled_back() {
    let pool = Pool::connect(":memory:", 1).unwrap();
    pool.execute("CREATE TABLE t (v INTEGER)", &[]).await.unwrap();

    {
        let conn = pool.get().unwrap();
        conn.begin().unwrap();
        conn.execute("INSERT INTO t (v) VALUES (?)", &[Value::Int(1)]).unwrap();
        // Visible on this connection while the transaction is open …
        assert_eq!(conn.query("SELECT COUNT(*) AS count FROM t", &[]).unwrap()[0]["count"], 1);
        // … and gone once the dropped `CheckedConn` has rolled back.
    }

    let rows = pool.query("SELECT COUNT(*) AS count FROM t", &[]).await.unwrap();
    assert_eq!(rows[0]["count"], 0);
}

#[test]
fn sqlite_status_counts_open_connections() {
    let pool = Pool::connect(":memory:", 4).unwrap();
    let _conn = pool.get().unwrap();
    assert!(pool.status().connections >= 1);
}
