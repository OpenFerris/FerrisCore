//! Live MySQL/MariaDB check for the dialect bridge.
//!
//! MySQL takes `?` like SQLite, so this test is not about placeholders: it
//! proves that the same `Db`/`Stmt` path, DDL and row decoding work against the
//! second production engine. It is skipped unless `FERRIS_TEST_MYSQL_HOST` is
//! set; CI starts the pinned service and sets the five variables.

#![allow(unused_crate_dependencies)]

use ferris_core::{DatabaseConfig, DatabaseType, SslMode, Value};
use ferris_db::{Db, Stmt};

fn config() -> Option<DatabaseConfig> {
    let host = std::env::var("FERRIS_TEST_MYSQL_HOST").ok()?;
    Some(DatabaseConfig {
        r#type: DatabaseType::MySQL,
        host,
        port: std::env::var("FERRIS_TEST_MYSQL_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(3306),
        name: std::env::var("FERRIS_TEST_MYSQL_DB").unwrap_or_else(|_| "ferris_test".to_string()),
        user: std::env::var("FERRIS_TEST_MYSQL_USER").unwrap_or_else(|_| "root".to_string()),
        password: std::env::var("FERRIS_TEST_MYSQL_PASSWORD")
            .unwrap_or_else(|_| "test".to_string()),
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
async fn a_mysql_statement_with_binds_round_trips() {
    let Some(config) = config() else {
        eprintln!("skipping: FERRIS_TEST_MYSQL_HOST is not set");
        return;
    };

    let db = Db::connect(&config).await.expect("mysql connects");
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
