// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "sqlite")]
//! Round-3 functional acceptance on in-memory SQLite: timestamps, soft delete,
//! hooks, `insert_many` (including the 999-parameter chunk boundary) and
//! aggregates (§23–§27, §31).
use std::sync::Mutex;
use std::time::Duration;

use bee_orm::pool::sqlite::Pool;
use bee_orm::{Model, OrmError, Value};

async fn setup(ddl: &str) -> Pool {
    let pool = Pool::connect(":memory:", 4).unwrap();
    pool.execute(ddl, &[]).await.unwrap();
    pool
}

// ---------------------------------------------------------------- timestamps

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "notes")]
struct Note {
    #[bee(pk, auto)]
    id: i64,
    body: String,
    #[bee(auto_now_add)]
    created_at: i64,
    #[bee(auto_now)]
    updated_at: i64,
}

async fn setup_notes() -> Pool {
    setup(
        "CREATE TABLE notes (id INTEGER PRIMARY KEY AUTOINCREMENT, body TEXT NOT NULL, \
         created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
    )
    .await
}

#[tokio::test]
async fn timestamps_fill_on_insert_and_refresh_only_auto_now_on_update() {
    let pool = setup_notes().await;
    let mut note = Note { id: 0, body: "hello".into(), created_at: 0, updated_at: 0 };
    // The trait injects "now"; the struct's own values are never written.
    assert!(!note.insert_values().iter().any(|(column, _)| *column == "created_at"));
    assert!(!note.insert_values().iter().any(|(column, _)| *column == "updated_at"));

    assert_eq!(note.insert(&pool).await.unwrap(), 1);
    let stored = Note::query().one(&pool).await.unwrap().unwrap();
    assert!(stored.created_at > 0, "created_at must be a real unix second");
    // One "now" per statement, so both columns carry it.
    assert_eq!(stored.created_at, stored.updated_at);
    let (created, updated) = (stored.created_at, stored.updated_at);

    // Unix seconds: let the clock tick so a refreshed value is visible.
    std::thread::sleep(Duration::from_millis(1100));
    note.id = stored.id;
    note.body = "edited".into();
    assert_eq!(note.update(&pool).await.unwrap(), 1);

    let reread = Note::query().one(&pool).await.unwrap().unwrap();
    assert_eq!(reread.body, "edited");
    assert_eq!(reread.created_at, created, "update must not touch auto_now_add");
    assert!(reread.updated_at > updated, "update must refresh auto_now");
    // The in-memory struct stays stale until re-read.
    assert_eq!(note.updated_at, 0);
}

// -------------------------------------------------------------- soft delete

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "tasks")]
struct Task {
    #[bee(pk, auto)]
    id: i64,
    title: String,
    #[bee(soft_delete)]
    deleted: bool,
}

async fn setup_tasks() -> Pool {
    setup(
        "CREATE TABLE tasks (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, \
         deleted INTEGER NOT NULL)",
    )
    .await
}

#[tokio::test]
async fn soft_delete_hides_the_row_and_the_escape_hatches_see_it() {
    let pool = setup_tasks().await;
    let task = Task { id: 0, title: "write".into(), deleted: false };
    assert_eq!(task.insert(&pool).await.unwrap(), 1);
    let stored = Task::query().one(&pool).await.unwrap().unwrap();

    let row = stored.clone();
    assert_eq!(row.delete(&pool).await.unwrap(), 1);
    // Invisible through every default path …
    assert!(Task::query().all(&pool).await.unwrap().is_empty());
    assert!(Task::query().one(&pool).await.unwrap().is_none());
    assert_eq!(Task::query().count(&pool).await.unwrap(), 0);
    assert!(!Task::query().exists(&pool).await.unwrap());
    // … and deleting again changes nothing (the flag term matches no row).
    assert_eq!(row.delete(&pool).await.unwrap(), 0);

    // with_deleted() is the escape hatch.
    let seen = Task::query().with_deleted().all(&pool).await.unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].deleted);
    assert_eq!(Task::query().with_deleted().count(&pool).await.unwrap(), 1);

    // Instance update is pk-addressed: it touches the deleted row (restore path).
    let mut restored = seen[0].clone();
    restored.title = "write more".into();
    assert_eq!(restored.update(&pool).await.unwrap(), 1);
    let reread = Task::query().with_deleted().one(&pool).await.unwrap().unwrap();
    assert_eq!(reread.title, "write more");

    // hard_delete() removes it for real.
    assert_eq!(reread.hard_delete(&pool).await.unwrap(), 1);
    assert!(Task::query().with_deleted().all(&pool).await.unwrap().is_empty());
}

// -------------------------------------------------------------------- hooks

/// Hook call order across every operation, one shared log (single test).
static HOOKS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn take_hooks() -> Vec<String> {
    std::mem::take(&mut *HOOKS.lock().unwrap())
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(
    table = "audited",
    hooks(before_insert, after_insert, before_update, after_update, before_delete, after_delete)
)]
struct Audited {
    #[bee(pk, auto)]
    id: i64,
    label: String,
    /// Rejects the insert from `before_insert`.
    #[bee(ignore)]
    blocked: bool,
}

impl Audited {
    async fn before_insert(&self) -> bee_orm::Result<()> {
        HOOKS.lock().unwrap().push(format!("before_insert:{}", self.label));
        if self.blocked {
            return Err(OrmError::QueryError("blocked by before_insert".into()));
        }
        Ok(())
    }

    async fn after_insert(&self) -> bee_orm::Result<()> {
        HOOKS.lock().unwrap().push(format!("after_insert:{}", self.label));
        Ok(())
    }

    async fn before_update(&self) -> bee_orm::Result<()> {
        HOOKS.lock().unwrap().push(format!("before_update:{}", self.label));
        Ok(())
    }

    async fn after_update(&self) -> bee_orm::Result<()> {
        HOOKS.lock().unwrap().push(format!("after_update:{}", self.label));
        Ok(())
    }

    async fn before_delete(&self) -> bee_orm::Result<()> {
        HOOKS.lock().unwrap().push(format!("before_delete:{}", self.label));
        Ok(())
    }

    async fn after_delete(&self) -> bee_orm::Result<()> {
        HOOKS.lock().unwrap().push(format!("after_delete:{}", self.label));
        Ok(())
    }
}

async fn setup_audited() -> Pool {
    setup("CREATE TABLE audited (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL)").await
}

#[tokio::test]
async fn hooks_fire_in_order_and_a_before_error_aborts_before_any_write() {
    let pool = setup_audited().await;
    take_hooks();

    let mut row = Audited { id: 0, label: "ok".into(), blocked: false };
    assert_eq!(row.insert(&pool).await.unwrap(), 1);
    assert_eq!(take_hooks(), vec!["before_insert:ok", "after_insert:ok"]);
    // The auto pk is not written back into the in-memory struct, so the
    // pk-addressed update/delete below need the id re-read from the row.
    row.id = Audited::query().one(&pool).await.unwrap().unwrap().id;

    row.label = "edited".into();
    assert_eq!(row.update(&pool).await.unwrap(), 1);
    assert_eq!(take_hooks(), vec!["before_update:edited", "after_update:edited"]);

    assert_eq!(row.delete(&pool).await.unwrap(), 1);
    assert_eq!(take_hooks(), vec!["before_delete:edited", "after_delete:edited"]);
    assert_eq!(Audited::query().count(&pool).await.unwrap(), 0);

    // A failing `before_*` aborts before any SQL runs and skips every `after_*`.
    let blocked = Audited { id: 0, label: "blocked".into(), blocked: true };
    let err = blocked.insert(&pool).await.unwrap_err();
    assert!(
        matches!(&err, OrmError::QueryError(message) if message.contains("blocked by before_insert")),
        "unexpected error: {err}"
    );
    assert_eq!(Audited::query().count(&pool).await.unwrap(), 0, "no row may be written");
    assert_eq!(take_hooks(), vec!["before_insert:blocked"]);
}

// -------------------------------------------------------------- insert_many

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "points")]
struct Point {
    #[bee(pk, auto)]
    id: i64,
    v: i32,
}

async fn setup_points() -> Pool {
    setup("CREATE TABLE points (id INTEGER PRIMARY KEY AUTOINCREMENT, v INTEGER NOT NULL)").await
}

#[tokio::test]
async fn insert_many_round_trips_every_row() {
    let pool = setup_points().await;
    let rows: Vec<Point> = (0..3).map(|v| Point { id: 0, v }).collect();
    assert_eq!(Point::insert_many(&pool, &rows).await.unwrap(), 3);

    let stored = Point::query().order_by("v").all(&pool).await.unwrap();
    assert_eq!(stored.iter().map(|point| point.v).collect::<Vec<_>>(), vec![0, 1, 2]);
    assert!(stored.iter().all(|point| point.id > 0), "the auto pk is assigned per row");

    // An empty batch is a no-op, not an error.
    assert_eq!(Point::insert_many(&pool, &[]).await.unwrap(), 0);
    assert_eq!(Point::query().count(&pool).await.unwrap(), 3);
}

#[tokio::test]
async fn insert_many_chunks_past_the_999_parameter_floor() {
    const ROWS: i32 = 40_000;
    let pool = setup_points().await;

    // One column × 40 000 rows: a single statement would bind 40 000
    // parameters, which this backend refuses (control below), so this only
    // passes if `insert_many` really chunks (999 per statement, 41 statements).
    let rows: Vec<Point> = (0..ROWS).map(|v| Point { id: 0, v }).collect();
    assert_eq!(Point::insert_many(&pool, &rows).await.unwrap(), ROWS as u64);
    assert_eq!(Point::query().count(&pool).await.unwrap(), i64::from(ROWS));
    assert_eq!(Point::query().filter_eq("v", ROWS - 1).unwrap().count(&pool).await.unwrap(), 1);

    // Control: the same shape as one un-chunked statement is rejected, so the
    // assertions above would fail if the chunking were removed.
    let sql = format!("INSERT INTO points (v) VALUES {}", vec!["(?)"; ROWS as usize].join(", "));
    let params = vec![Value::Int(0); ROWS as usize];
    assert!(pool.execute(&sql, &params).await.is_err(), "the un-chunked statement must not fit");
}

// --------------------------------------------------------------- aggregates

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "scores")]
struct Score {
    #[bee(pk, auto)]
    id: i64,
    value: i32,
}

async fn setup_scores() -> Pool {
    setup("CREATE TABLE scores (id INTEGER PRIMARY KEY AUTOINCREMENT, value INTEGER NOT NULL)")
        .await
}

#[tokio::test]
async fn aggregates_compute_over_the_fixture_and_reject_invalid_fields() {
    let pool = setup_scores().await;
    let rows: Vec<Score> = [1, 2, 3].into_iter().map(|value| Score { id: 0, value }).collect();
    Score::insert_many(&pool, &rows).await.unwrap();

    let all = Score::query();
    assert_eq!(all.sum(&pool, "value").await.unwrap(), Some(6.0));
    assert_eq!(all.avg(&pool, "value").await.unwrap(), Some(2.0));
    assert_eq!(all.min(&pool, "value").await.unwrap(), Some(serde_json::json!(1)));
    assert_eq!(all.max(&pool, "value").await.unwrap(), Some(serde_json::json!(3)));

    // An empty result set is SQL NULL — `None`, not an error.
    let none = Score::query().filter_gt("value", 100).unwrap();
    assert_eq!(none.sum(&pool, "value").await.unwrap(), None);
    assert_eq!(none.avg(&pool, "value").await.unwrap(), None);
    assert_eq!(none.min(&pool, "value").await.unwrap(), None);
    assert_eq!(none.max(&pool, "value").await.unwrap(), None);

    // The field goes through the same validation as filters: a spliced
    // statement is rejected before any SQL is built. Validation is syntactic —
    // the ORM has no schema, so an unknown-but-well-formed name reaches the
    // backend instead of failing here.
    let field = "value; DROP TABLE scores";
    assert!(matches!(Score::query().sum(&pool, field).await, Err(OrmError::InvalidField(_))));
    assert!(matches!(Score::query().avg(&pool, field).await, Err(OrmError::InvalidField(_))));
    assert!(matches!(Score::query().min(&pool, field).await, Err(OrmError::InvalidField(_))));
    assert!(matches!(Score::query().max(&pool, field).await, Err(OrmError::InvalidField(_))));
    // The rejected fields were never executed.
    assert_eq!(Score::query().count(&pool).await.unwrap(), 3);
}
