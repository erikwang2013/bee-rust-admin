// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "mysql")]
//! §44/§47 tester falsification of the mysql table-level FK opt-in:
//! the constraint is real (checked semantically, not by name — the CREATE-path
//! clause is unnamed DDL), re-runs are no-ops, the default path keeps the
//! round-4 ignore pin — and the orphan failure leaves a state whose documented
//! "clean the data" remedy is measured below.
//!
//! Env-gated by `BEE_ORM_MYSQL_DSN` like the other mysql targets; each test
//! owns its table set (no cross-test DROP races).
mod common;

use bee_orm::pool::mysql::Pool;
use bee_orm::{MigrateOptions, Model, OrmError, Value, migrate};

const ON: MigrateOptions = MigrateOptions { table_level_fk: true };

/// The `information_schema` names of a table's foreign-key constraints, sorted.
async fn fk_constraints(pool: &Pool, table: &str) -> Result<Vec<String>, OrmError> {
    let rows = pool
        .query(
            "SELECT constraint_name AS name FROM information_schema.table_constraints \
             WHERE table_schema = DATABASE() AND table_name = ? \
             AND constraint_type = 'FOREIGN KEY'",
            &[Value::Text(table.into())],
        )
        .await?;
    let mut names: Vec<String> =
        rows.iter().filter_map(|row| row["name"].as_str().map(String::from)).collect();
    names.sort();
    Ok(names)
}

/// The single foreign-key reference of `table.column` as `(table, column)`.
/// Semantic, not name-based: the CREATE-path clause is unnamed in the DDL, so
/// mysql auto-names it `{table}_ibfk_N` (§44's documented `{table}_{column}_fk`
/// name belongs to the ADD CONSTRAINT path, covered by the orphan test).
async fn fk_reference(
    pool: &Pool,
    table: &str,
    column: &str,
) -> Result<(String, String), OrmError> {
    let rows = pool
        .query(
            "SELECT referenced_table_name AS tbl, referenced_column_name AS col \
             FROM information_schema.key_column_usage \
             WHERE table_schema = DATABASE() AND table_name = ? AND column_name = ? \
             AND referenced_table_name IS NOT NULL",
            &[Value::Text(table.into()), Value::Text(column.into())],
        )
        .await?;
    assert_eq!(rows.len(), 1, "expected exactly one FK reference on {table}.{column}");
    Ok((
        rows[0]["tbl"].as_str().unwrap_or_default().to_string(),
        rows[0]["col"].as_str().unwrap_or_default().to_string(),
    ))
}

/// The column names of a table, sorted.
async fn columns(pool: &Pool, table: &str) -> Result<Vec<String>, OrmError> {
    let rows = pool
        .query(
            "SELECT column_name AS name FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = ?",
            &[Value::Text(table.into())],
        )
        .await?;
    let mut names: Vec<String> =
        rows.iter().filter_map(|row| row["name"].as_str().map(String::from)).collect();
    names.sort();
    Ok(names)
}

// ------------------------------------------------ fresh opt-in + idempotency

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_fk_teams")]
struct FkTeam {
    #[bee(pk, auto)]
    id: i64,
    name: String,
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_fk_members")]
struct FkMember {
    #[bee(pk, auto)]
    id: i64,
    #[bee(fk = FkTeam)]
    team_id: i64,
    name: String,
}

/// Same fk shape, default path: the control for "opt-in changes only mysql,
/// only when asked" (round-4 pin).
#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_fk_plain")]
struct FkPlainMember {
    #[bee(pk, auto)]
    id: i64,
    #[bee(fk = FkTeam)]
    team_id: i64,
}

#[tokio::test]
async fn opt_in_enforces_once_and_re_runs_are_no_ops() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_fk_plain", &[]).await?;
    pool.execute("DROP TABLE IF EXISTS it_fk_members", &[]).await?;
    pool.execute("DROP TABLE IF EXISTS it_fk_teams", &[]).await?;

    migrate::create_table_with::<FkTeam, _>(&pool, ON).await?;
    migrate::create_table_with::<FkMember, _>(&pool, ON).await?;
    let created = fk_constraints(&pool, "it_fk_members").await?;
    assert_eq!(created.len(), 1, "the opt-in renders one table-level constraint: {created:?}");
    assert_eq!(
        fk_reference(&pool, "it_fk_members", "team_id").await?,
        ("it_fk_teams".to_string(), "id".to_string())
    );

    // Enforcement is real: this dangling insert is the same one the control
    // below lands successfully.
    pool.execute("INSERT INTO it_fk_teams (id, name) VALUES (100, 'explicit')", &[]).await?;
    let dangling = FkMember { id: 0, team_id: 999_999, name: "ghost".into() };
    assert!(dangling.insert(&pool).await.is_err(), "opted-in mysql must enforce the FK");
    assert_eq!(FkMember { id: 0, team_id: 100, name: "root".into() }.insert(&pool).await?, 1);

    // Idempotency, name-matched: column present → nothing added, no duplicate
    // constraint, no error — twice, and through `sync_with` as well.
    assert_eq!(migrate::add_missing_columns_with::<FkMember, _>(&pool, ON).await?, 0);
    assert_eq!(migrate::add_missing_columns_with::<FkMember, _>(&pool, ON).await?, 0);
    assert_eq!(migrate::sync_with::<FkMember, _>(&pool, ON).await?, 0);
    assert_eq!(migrate::sync_with::<FkMember, _>(&pool, ON).await?, 0);
    assert_eq!(
        fk_constraints(&pool, "it_fk_members").await?,
        created,
        "re-runs must not duplicate the constraint"
    );

    // Control: the default path still parses-and-ignores the inline
    // `REFERENCES` and takes the dangling row (round-4 pin, §44 default off).
    migrate::create_table::<FkPlainMember, _>(&pool).await?;
    assert!(fk_constraints(&pool, "it_fk_plain").await?.is_empty());
    assert_eq!(FkPlainMember { id: 0, team_id: 999_999 }.insert(&pool).await?, 1);
    Ok(())
}

// ------------------------------------------- orphan failure + clean-then-rerun

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_fk_o_teams")]
struct FkOTeam {
    #[bee(pk, auto)]
    id: i64,
    name: String,
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "it_fk_o_members")]
struct FkOMember {
    #[bee(pk, auto)]
    id: i64,
    #[bee(fk = FkOTeam)]
    team_id: i64,
    name: String,
}

/// The ADD path over pre-existing rows: mysql backfills the new NOT NULL
/// column (`team_id = 0`, no such team), so the `ADD CONSTRAINT` hits orphans
/// and must propagate the driver error as `QueryError`.
#[tokio::test]
async fn opt_in_over_orphans_errors_and_the_remedy_is_a_manual_alter() -> Result<(), OrmError> {
    let Some(dsn) = common::dsn("BEE_ORM_MYSQL_DSN") else { return Ok(()) };
    let pool = Pool::connect(&dsn, 4)?;
    pool.execute("DROP TABLE IF EXISTS it_fk_o_members", &[]).await?;
    pool.execute("DROP TABLE IF EXISTS it_fk_o_teams", &[]).await?;

    migrate::create_table_with::<FkOTeam, _>(&pool, ON).await?;
    pool.execute("INSERT INTO it_fk_o_teams (id, name) VALUES (100, 'explicit')", &[]).await?;
    // The child predates the fk column: populated, no `team_id` yet.
    pool.execute(
        "CREATE TABLE it_fk_o_members (id BIGINT AUTO_INCREMENT PRIMARY KEY, \
         name VARCHAR(255) NOT NULL)",
        &[],
    )
    .await?;
    pool.execute("INSERT INTO it_fk_o_members (name) VALUES ('old')", &[]).await?;

    let err = match migrate::add_missing_columns_with::<FkOMember, _>(&pool, ON).await {
        Ok(added) => panic!("ADD CONSTRAINT over orphans must fail, but added {added}"),
        Err(err) => err,
    };
    assert!(matches!(err, OrmError::QueryError(_)), "unexpected error: {err}");

    // The failure is half-applied by design: the column stays, the constraint
    // does not exist.
    assert!(columns(&pool, "it_fk_o_members").await?.contains(&"team_id".to_string()));
    assert!(fk_constraints(&pool, "it_fk_o_members").await?.is_empty());

    // MEASURED (reported): cleaning the orphans and re-running does *not*
    // converge — the now-existing column is skipped before the name-matched
    // constraint check ever runs, so this assertion pins current behavior.
    pool.execute("UPDATE it_fk_o_members SET team_id = 100", &[]).await?;
    assert_eq!(migrate::add_missing_columns_with::<FkOMember, _>(&pool, ON).await?, 0);
    assert!(
        fk_constraints(&pool, "it_fk_o_members").await?.is_empty(),
        "measured: a re-run after cleaning does not add the constraint"
    );

    // The effective remedy is the manual ALTER — same name, same shape, and
    // the constraint is enforced once it exists.
    pool.execute(
        "ALTER TABLE it_fk_o_members ADD CONSTRAINT it_fk_o_members_team_id_fk \
         FOREIGN KEY (team_id) REFERENCES it_fk_o_teams (id)",
        &[],
    )
    .await?;
    assert_eq!(
        fk_constraints(&pool, "it_fk_o_members").await?,
        vec!["it_fk_o_members_team_id_fk".to_string()]
    );
    let dangling = FkOMember { id: 0, team_id: 999_999, name: "ghost".into() };
    assert!(dangling.insert(&pool).await.is_err(), "the manual constraint must be enforced");
    Ok(())
}
