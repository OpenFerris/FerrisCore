//! Live PostgreSQL check for the `?` → `$n` bridge.
//!
//! The framework writes `?` because it is the portable spelling; `ferris-db`
//! renders it as `$1`, `$2`, … for Postgres. That translation is the only
//! reason a bound statement can reach Postgres at all, so it is exercised
//! against a real server instead of being assumed.
//!
//! The test is skipped unless `FERRIS_TEST_PG_HOST` is set. CI starts the
//! pinned Postgres service and sets the five `FERRIS_TEST_PG_*` variables.

// The crate's remaining dependencies are used by the library, not this binary.
#![allow(unused_crate_dependencies)]

use ferris_core::{DatabaseConfig, DatabaseType, SslMode, Value};
use ferris_db::{Db, Stmt};

fn config() -> Option<DatabaseConfig> {
    let host = std::env::var("FERRIS_TEST_PG_HOST").ok()?;
    Some(DatabaseConfig {
        r#type: DatabaseType::Postgres,
        host,
        port: std::env::var("FERRIS_TEST_PG_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5432),
        name: std::env::var("FERRIS_TEST_PG_DB").unwrap_or_else(|_| "ferris_test".to_string()),
        user: std::env::var("FERRIS_TEST_PG_USER").unwrap_or_else(|_| "test".to_string()),
        password: std::env::var("FERRIS_TEST_PG_PASSWORD").unwrap_or_else(|_| "test".to_string()),
        max_connections: 2,
        min_connections: 0,
        connect_timeout_secs: 10,
        idle_timeout_secs: 30,
        max_lifetime_secs: 300,
        ssl_mode: SslMode::Disable,
        schema: None,
    })
}

#[tokio::test]
async fn a_postgres_statement_with_binds_round_trips() {
    let Some(config) = config() else {
        eprintln!("skipping: FERRIS_TEST_PG_HOST is not set");
        return;
    };

    let db = Db::connect(&config).await.expect("postgres connects");
    db.execute_script("DROP TABLE IF EXISTS tab_review_probe")
        .await
        .expect("drop probe table");
    db.execute_script(
        "CREATE TABLE tab_review_probe (\
         name VARCHAR(255) PRIMARY KEY, \
         owner VARCHAR(255), \
         docstatus SMALLINT NOT NULL DEFAULT 0, \
         creation BIGINT NOT NULL, \
         modified BIGINT NOT NULL, \
         data TEXT NOT NULL)",
    )
    .await
    .expect("create probe table");

    db.execute(&Stmt::new(
        "INSERT INTO tab_review_probe (name, owner, docstatus, creation, modified, data) \
         VALUES (?, ?, ?, ?, ?, ?)",
        vec![
            Value::from("probe"),
            Value::from("tester"),
            Value::from(1_i64),
            Value::from(10_i64),
            Value::from(10_i64),
            Value::from(r#"{"name":"probe"}"#),
        ],
    ))
    .await
    .expect("insert with binds");

    let row = db
        .fetch_optional(&Stmt::new(
            "SELECT data FROM tab_review_probe WHERE name = ? AND docstatus = ?",
            vec![Value::from("probe"), Value::from(1_i64)],
        ))
        .await
        .expect("select with binds")
        .expect("the inserted row");
    assert_eq!(row.str("data").expect("data column"), r#"{"name":"probe"}"#);

    db.execute(&Stmt::new(
        "UPDATE tab_review_probe SET data = ? WHERE name = ?",
        vec![
            Value::from(r#"{"name":"probe","v":2}"#),
            Value::from("probe"),
        ],
    ))
    .await
    .expect("update with binds");

    let row = db
        .fetch_one(&Stmt::new(
            "SELECT data FROM tab_review_probe WHERE name = ?",
            vec![Value::from("probe")],
        ))
        .await
        .expect("read the update back");
    assert_eq!(
        row.str("data").expect("data column"),
        r#"{"name":"probe","v":2}"#
    );

    let removed = db
        .execute(&Stmt::new(
            "DELETE FROM tab_review_probe WHERE name = ?",
            vec![Value::from("probe")],
        ))
        .await
        .expect("delete with binds");
    assert_eq!(removed.rows(), 1);

    db.execute_script("DROP TABLE tab_review_probe")
        .await
        .expect("cleanup");
}
