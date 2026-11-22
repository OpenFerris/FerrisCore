//! Unit tests for the migration runner.
//!
//! Restored from the inline `#[cfg(test)] mod tests` moved into `tests/`
//! by commit 2013f3d.

use super::*;
use crate::pool::open_unmanaged;
use ferris_core::{DatabaseConfig, DatabaseType};

fn memory_db() -> DatabaseConfig {
    DatabaseConfig {
        r#type: DatabaseType::SQLite,
        name: ":memory:".to_string(),
        max_connections: 1,
        min_connections: 0,
        ..DatabaseConfig::default()
    }
}

fn migration(version: u32, sql: &str) -> Migration {
    Migration::new(version, format!("m{version}"), sql, None)
}

#[test]
fn checksum_is_sha256_hex() {
    assert_eq!(
        calculate_checksum("CREATE TABLE test (id INT PRIMARY KEY)").len(),
        64
    );
    // Same SQL, same checksum; different SQL, different checksum.
    assert_eq!(calculate_checksum("a"), calculate_checksum("a"));
    assert_ne!(calculate_checksum("a"), calculate_checksum("b"));
}

#[test]
fn migration_new_computes_the_checksum() {
    let m = migration(1, "CREATE TABLE t (id INTEGER)");
    assert_eq!(
        m.checksum,
        calculate_checksum("CREATE TABLE t (id INTEGER)")
    );
}

#[test]
fn a_runner_rejects_out_of_order_versions() {
    let mut runner = MigrationRunner::new();
    runner.add_migration(migration(2, "SELECT 1"));
    runner.add_migration(migration(1, "SELECT 1"));
    assert!(runner.validate().is_err());

    let mut runner = MigrationRunner::new();
    runner.add_migration(migration(1, "SELECT 1"));
    runner.add_migration(migration(1, "SELECT 1"));
    assert!(runner.validate().is_err());

    let mut runner = MigrationRunner::new();
    runner.add_migration(migration(1, "SELECT 1"));
    runner.add_migration(migration(2, "SELECT 1"));
    assert!(runner.validate().is_ok());
}

#[tokio::test]
async fn a_multi_statement_migration_runs_and_is_idempotent() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    let runner = MigrationRunner::with_migrations([migration(
        1,
        "CREATE TABLE t (id INTEGER PRIMARY KEY); CREATE INDEX idx_t ON t (id);",
    )]);

    let results = runner.migrate(&db, None).await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].success, "{:?}", results[0].error);

    // A second run finds nothing pending.
    assert!(runner.migrate(&db, None).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_edited_migration_is_rejected() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    let sql = "CREATE TABLE t (id INTEGER PRIMARY KEY);";
    MigrationRunner::with_migrations([migration(1, sql)])
        .migrate(&db, None)
        .await
        .unwrap();

    let tampered = Migration {
        checksum: "deadbeef".to_string(),
        ..migration(1, sql)
    };
    let runner = MigrationRunner::with_migrations([tampered]);

    assert!(runner.migrate(&db, None).await.is_err());
    assert_eq!(runner.checksum_mismatches(&db).await.unwrap().len(), 1);

    // Forcing applies past it instead.
    assert!(runner.migrate_forced(&db, None).await.is_ok());
}

#[tokio::test]
async fn pending_excludes_what_was_already_applied() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    let runner = MigrationRunner::with_migrations([
        migration(1, "CREATE TABLE a (id INTEGER PRIMARY KEY);"),
        migration(2, "CREATE TABLE b (id INTEGER PRIMARY KEY);"),
    ]);

    assert_eq!(runner.pending(&db).await.unwrap().len(), 2);
    runner.migrate(&db, Some(1)).await.unwrap();
    let pending = runner.pending(&db).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].version, 2);
}

#[tokio::test]
async fn a_failing_migration_is_reported_and_not_recorded() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    let runner = MigrationRunner::with_migrations([
        migration(1, "CREATE TABLE a (id INTEGER PRIMARY KEY);"),
        migration(2, "THIS IS NOT SQL;"),
    ]);

    let results = runner.migrate(&db, None).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].success);
    assert!(!results[1].success);
    assert!(results[1].error.is_some());

    // The failed migration is still pending, and the successful one is not.
    let pending = runner.pending(&db).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].version, 2);
}

#[tokio::test]
async fn bookkeeping_is_idempotent() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    ensure_bookkeeping(&db).await.unwrap();
    ensure_bookkeeping(&db).await.unwrap();
}

/// The whole batch runs on one pinned connection.
///
/// A pool of exactly one connection is the sharpest form of the regression:
/// if the runner acquired a connection for the lock and then tried to check
/// out a second one to read the applied set or to run a migration, it would
/// block on itself and surface as `PoolExhausted` (which is exactly what it
/// did before the lock and the reads were moved onto the pinned session).
#[tokio::test]
async fn the_batch_runs_on_a_single_pinned_connection() {
    let db = open_unmanaged(&memory_db()).await.unwrap();

    let runner = MigrationRunner::with_migrations([
        migration(1, "CREATE TABLE a (id INTEGER PRIMARY KEY);"),
        migration(2, "CREATE TABLE b (id INTEGER PRIMARY KEY);"),
    ]);
    let results = runner.migrate(&db, None).await.unwrap();
    assert_eq!(results.len(), 2);
    for result in &results {
        assert!(result.success, "{:?}", result.error);
    }
    // Both recorded, so a second run is a no-op.
    assert!(runner.migrate(&db, None).await.unwrap().is_empty());
}

/// H11: `GET_LOCK` has three outcomes and only 1 is success. The result used
/// to be discarded, so a timeout (0) or an error (NULL) continued as if the
/// process held the lock.
#[test]
fn mysql_lock_result_is_checked() {
    assert!(check_mysql_lock(Some(1)).is_ok());
    for refused in [Some(0), None] {
        let err = check_mysql_lock(refused).expect_err("0/NULL must not be treated as held");
        let message = err.to_string();
        assert!(message.contains("GET_LOCK returned"), "{message}");
    }
}
