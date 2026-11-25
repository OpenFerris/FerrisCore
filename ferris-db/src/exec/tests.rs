//! Unit tests for the execution boundary.
//!
//! Restored from the inline `#[cfg(test)] mod tests` that commit 2013f3d
//! moved into `tests/`, where `crate::pool::…` / `crate::sql::…` stopped
//! naming this crate.

use super::*;
use crate::dialect::Dialect;
use crate::pool::open_unmanaged;
use crate::sql::{Column, UpsertBuilder, UpsertSet};
use ferris_core::{DatabaseConfig, DatabaseType, Value};

fn memory_db() -> DatabaseConfig {
    DatabaseConfig {
        r#type: DatabaseType::SQLite,
        name: ":memory:".to_string(),
        max_connections: 1,
        min_connections: 0,
        ..DatabaseConfig::default()
    }
}

fn col(name: &str, ty: &str) -> Column {
    Column::new(name, ty)
}

fn key(name: &str, ty: &str) -> Column {
    col(name, ty).primary_key()
}

#[tokio::test]
async fn values_are_bound_not_interpolated() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.ensure_table("t", &[col("name", "VARCHAR(255)"), col("payload", "TEXT")])
        .await
        .unwrap();

    // A value carrying SQL is stored verbatim, which only holds if it was
    // bound as a parameter.
    let hostile = "'; DROP TABLE t; --";
    db.execute(&Stmt::new(
        "INSERT INTO t (name, payload) VALUES (?, ?)",
        vec![Value::from(hostile), Value::from("x")],
    ))
    .await
    .unwrap();

    let row = db
        .fetch_one(&Stmt::new(
            "SELECT name FROM t WHERE payload = ?",
            vec![Value::from("x")],
        ))
        .await
        .unwrap();
    assert_eq!(row.str("name").unwrap(), hostile);
    assert_eq!(db.count(&Stmt::raw("SELECT * FROM t")).await.unwrap(), 1);
}

#[tokio::test]
async fn fetch_optional_returns_none_instead_of_failing() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    let stmt = Stmt::raw("SELECT 1 AS one WHERE 1 = 0");
    assert!(db.fetch_optional(&stmt).await.unwrap().is_none());
    // The strict variant is the one that reports the empty result.
    assert!(db.fetch_one(&stmt).await.is_err());
}

#[tokio::test]
async fn count_counts_groups_and_the_unpaged_total() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.ensure_table("t", &[col("name", "VARCHAR(255)")])
        .await
        .unwrap();
    for i in 0..5 {
        db.execute(&Stmt::new(
            "INSERT INTO t (name) VALUES (?)",
            vec![Value::from(format!("T-{}", i % 2))],
        ))
        .await
        .unwrap();
    }
    assert_eq!(db.count(&Stmt::raw("SELECT name FROM t")).await.unwrap(), 5);
    // The sub-select is why a grouped query counts groups instead of
    // collapsing to a single number.
    assert_eq!(
        db.count(&Stmt::raw("SELECT name, COUNT(*) FROM t GROUP BY name"))
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn ensure_table_and_index_are_idempotent() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    let table = "t";
    for _ in 0..2 {
        db.ensure_table(&table, &[col("name", "VARCHAR(255)")])
            .await
            .unwrap();
        db.ensure_index("idx_t_name", &table, &["name"], false)
            .await
            .unwrap();
    }
}

/// `ensure_column` migrates an existing table and is idempotent across
/// boots — the MFA columns on `users` depend on it.
#[tokio::test]
async fn ensure_column_adds_and_is_idempotent() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.ensure_table("t", &[col("name", "VARCHAR(255)")])
        .await
        .unwrap();
    let mfa = col("mfa_enabled", "INTEGER").not_null().default("0");
    db.ensure_column("t", &mfa).await.unwrap();
    // Second run must tolerate the existing column.
    db.ensure_column("t", &mfa).await.unwrap();
    // A table that already had the column at create time tolerates it too.
    db.ensure_column("t", &mfa).await.unwrap();
    // Rows that predate the migration pick up the new column's DEFAULT.
    db.execute(&Stmt::raw("INSERT INTO t (name) VALUES ('r1')"))
        .await
        .unwrap();
    let row = db
        .fetch_one(&Stmt::raw("SELECT mfa_enabled FROM t"))
        .await
        .unwrap();
    assert_eq!(row.i64("mfa_enabled").unwrap(), 0);
}

#[tokio::test]
async fn upsert_inserts_then_updates_in_place() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.ensure_table(
        "sessions",
        &[key("id", "VARCHAR(255)"), col("data", "TEXT")],
    )
    .await
    .unwrap();

    let make = |data: &str| {
        UpsertBuilder::new(db.dialect(), "sessions")
            .values(vec![("id", Value::from("s1")), ("data", Value::from(data))])
            .conflict_on(["id"])
            .on_update("data", UpsertSet::Value(Value::from(data)))
            .build()
            .unwrap()
    };

    db.upsert(make("first")).await.unwrap();
    assert_eq!(
        db.count(&Stmt::raw("SELECT * FROM sessions"))
            .await
            .unwrap(),
        1
    );

    db.upsert(make("second")).await.unwrap();
    assert_eq!(
        db.count(&Stmt::raw("SELECT * FROM sessions"))
            .await
            .unwrap(),
        1
    );

    let row = db
        .fetch_one(&Stmt::raw("SELECT data FROM sessions WHERE id = 's1'"))
        .await
        .unwrap();
    assert_eq!(row.str("data").unwrap(), "second");
}

#[tokio::test]
async fn a_counter_assignment_increments_in_the_database() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.ensure_table(
        "refs",
        &[key("id", "VARCHAR(255)"), col("version", "INTEGER")],
    )
    .await
    .unwrap();

    for _ in 0..3 {
        let upsert = UpsertBuilder::new(db.dialect(), "refs")
            .values(vec![("id", Value::from("f1")), ("version", Value::from(1))])
            .conflict_on(["id"])
            .on_update("version", UpsertSet::Increment(Value::from(1)))
            .build()
            .unwrap();
        db.upsert(upsert).await.unwrap();
    }

    let row = db
        .fetch_one(&Stmt::raw("SELECT version FROM refs WHERE id = 'f1'"))
        .await
        .unwrap();
    // The first call inserts 1; the other two hit the conflict and add 1
    // each. The increment happens in the database, so this is not a
    // read-modify-write the caller could lose a concurrent update to.
    assert_eq!(row.i64("version").unwrap(), 3);
}

#[tokio::test]
async fn a_transaction_commits_and_can_read_its_own_writes() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.ensure_table("t", &[col("name", "VARCHAR(255)")])
        .await
        .unwrap();

    let mut tx = db.begin().await.unwrap();
    tx.execute(&Stmt::new(
        "INSERT INTO t (name) VALUES (?)",
        vec![Value::from("T-1")],
    ))
    .await
    .unwrap();
    let row = tx
        .fetch_one(&Stmt::raw("SELECT name FROM t WHERE name = 'T-1'"))
        .await
        .unwrap();
    assert_eq!(row.str("name").unwrap(), "T-1");
    tx.commit().await.unwrap();

    assert_eq!(db.count(&Stmt::raw("SELECT * FROM t")).await.unwrap(), 1);
}

#[tokio::test]
async fn a_rolled_back_transaction_leaves_nothing_behind() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.ensure_table("t", &[col("name", "VARCHAR(255)")])
        .await
        .unwrap();

    let mut tx = db.begin().await.unwrap();
    tx.execute(&Stmt::new(
        "INSERT INTO t (name) VALUES (?)",
        vec![Value::from("T-1")],
    ))
    .await
    .unwrap();
    tx.rollback().await.unwrap();

    assert_eq!(db.count(&Stmt::raw("SELECT * FROM t")).await.unwrap(), 0);
}

#[tokio::test]
async fn a_finished_transaction_refuses_further_work() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    let mut tx = db.begin().await.unwrap();
    tx.commit().await.unwrap();
    assert!(tx.is_finished());
    assert!(tx.rollback().await.is_err());
    assert!(
        tx.execute(&Stmt::raw("SELECT 1")).await.is_err(),
        "a committed transaction must not run more statements"
    );
}

#[tokio::test]
async fn a_dropped_transaction_is_rolled_back() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.ensure_table("t", &[col("name", "VARCHAR(255)")])
        .await
        .unwrap();

    {
        let mut tx = db.begin().await.unwrap();
        tx.execute(&Stmt::new(
            "INSERT INTO t (name) VALUES (?)",
            vec![Value::from("T-1")],
        ))
        .await
        .unwrap();
        // No commit: the handle goes out of scope here.
    }

    assert_eq!(db.count(&Stmt::raw("SELECT * FROM t")).await.unwrap(), 0);
}

#[tokio::test]
async fn a_unique_violation_is_classified_as_a_constraint_error() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.ensure_table("t", &[key("name", "VARCHAR(255)")])
        .await
        .unwrap();
    db.execute(&Stmt::new(
        "INSERT INTO t (name) VALUES (?)",
        vec![Value::from("dup")],
    ))
    .await
    .unwrap();

    let err = db
        .execute(&Stmt::new(
            "INSERT INTO t (name) VALUES (?)",
            vec![Value::from("dup")],
        ))
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            Error::Database(ferris_core::error::DatabaseError::Constraint(_))
        ),
        "expected a constraint error, got {err:?}"
    );
}

#[tokio::test]
async fn the_dialect_travels_with_the_connection() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    assert_eq!(db.dialect(), Dialect::SQLite);
    assert!(db.dialect().supports_on_conflict());
    assert!(!db.dialect().supports_ilike());
}

#[tokio::test]
async fn execute_script_runs_several_statements() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.execute_script(
        "CREATE TABLE a (id INTEGER); CREATE TABLE b (id INTEGER); CREATE INDEX ia ON a (id);",
    )
    .await
    .unwrap();
    assert_eq!(db.count(&Stmt::raw("SELECT * FROM a")).await.unwrap(), 0);
    assert_eq!(db.count(&Stmt::raw("SELECT * FROM b")).await.unwrap(), 0);
}

/// The pool keeps the concrete driver instead of being erased to `Any`.
#[tokio::test]
async fn the_pool_carries_the_concrete_backend() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    assert!(matches!(db.pool(), DbPool::Sqlite(_)));
    assert_eq!(db.dialect(), Dialect::SQLite);
}

/// Binding a bool and a float reaches the driver at their own Rust types
/// rather than as the lowered `Any` representation.
#[tokio::test]
async fn booleans_and_floats_bind_at_their_native_types() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    db.ensure_table(
        "measurements",
        &[
            col("id", "INTEGER"),
            col("enabled", "BOOLEAN"),
            col("amount", "REAL"),
        ],
    )
    .await
    .unwrap();

    db.execute(&Stmt::new(
        "INSERT INTO measurements (id, enabled, amount) VALUES (?, ?, ?)",
        vec![Value::from(1_i64), Value::from(true), Value::from(1.5_f64)],
    ))
    .await
    .unwrap();

    let row = db
        .fetch_one(&Stmt::raw("SELECT enabled, amount FROM measurements"))
        .await
        .unwrap();
    assert!(row.bool("enabled").unwrap());
    // 1.5 is exactly representable, so an exact comparison is meaningful.
    #[allow(clippy::float_cmp)]
    let amount = row.opt_f64("amount").unwrap();
    assert_eq!(amount, 1.5);
}

/// `fetch_scalar` decodes the first column through the concrete driver.
#[tokio::test]
async fn fetch_scalar_decodes_the_first_column() {
    let db = open_unmanaged(&memory_db()).await.unwrap();
    let number = db
        .fetch_scalar::<i64>(&Stmt::raw("SELECT 7"))
        .await
        .unwrap();
    assert_eq!(number, Some(7));
    let text = db
        .fetch_scalar::<String>(&Stmt::raw("SELECT 'seven'"))
        .await
        .unwrap();
    assert_eq!(text.as_deref(), Some("seven"));
    let absent = db
        .fetch_scalar::<i64>(&Stmt::raw("SELECT 7 WHERE 1 = 0"))
        .await
        .unwrap();
    assert_eq!(absent, None);
}
