//! Live PostgreSQL/MySQL check for the session store's DDL and user index.
//!
//! The cutover's only session table is `tab_session_store`. SQLite exercises it
//! in the unit suite; this proves the `VARCHAR` primary key, the `TEXT` payload,
//! the `BIGINT` expiry and the `(user_id)` index on the two production engines.
//! Skipped unless `FERRIS_TEST_{PG,MYSQL}_HOST` is set (CI sets both).

#![allow(unused_crate_dependencies)]

use ferris_auth::login::DbSessionStore;
use ferris_core::{DatabaseConfig, DatabaseType, SslMode, Value};
use ferris_db::{Db, Stmt};

fn configs() -> Vec<(&'static str, DatabaseConfig)> {
    let mut out = Vec::new();
    if let Ok(host) = std::env::var("FERRIS_TEST_PG_HOST") {
        out.push((
            "postgres",
            DatabaseConfig {
                r#type: DatabaseType::Postgres,
                host,
                port: std::env::var("FERRIS_TEST_PG_PORT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5432),
                name: std::env::var("FERRIS_TEST_PG_DB")
                    .unwrap_or_else(|_| "ferris_test".to_string()),
                user: std::env::var("FERRIS_TEST_PG_USER").unwrap_or_else(|_| "test".to_string()),
                password: std::env::var("FERRIS_TEST_PG_PASSWORD")
                    .unwrap_or_else(|_| "test".to_string()),
                max_connections: 2,
                min_connections: 0,
                connect_timeout_secs: 10,
                idle_timeout_secs: 30,
                max_lifetime_secs: 300,
                ssl_mode: SslMode::Disable,
                schema: None,
            },
        ));
    }
    if let Ok(host) = std::env::var("FERRIS_TEST_MYSQL_HOST") {
        out.push((
            "mysql",
            DatabaseConfig {
                r#type: DatabaseType::MySQL,
                host,
                port: std::env::var("FERRIS_TEST_MYSQL_PORT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(3306),
                name: std::env::var("FERRIS_TEST_MYSQL_DB")
                    .unwrap_or_else(|_| "ferris_test".to_string()),
                user: std::env::var("FERRIS_TEST_MYSQL_USER")
                    .unwrap_or_else(|_| "root".to_string()),
                password: std::env::var("FERRIS_TEST_MYSQL_PASSWORD")
                    .unwrap_or_else(|_| "test".to_string()),
                max_connections: 2,
                min_connections: 0,
                connect_timeout_secs: 10,
                idle_timeout_secs: 30,
                max_lifetime_secs: 300,
                ssl_mode: SslMode::Disable,
                schema: None,
            },
        ));
    }
    out
}

#[tokio::test]
async fn the_session_table_round_trips_on_the_live_engines() {
    let configs = configs();
    if configs.is_empty() {
        eprintln!("skipping: no live database configured");
        return;
    }

    for (label, config) in configs {
        let db = Db::connect(&config)
            .await
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        db.execute_script("DROP TABLE IF EXISTS tab_session_store")
            .await
            .unwrap_or_else(|error| panic!("{label} drop: {error}"));

        let store = DbSessionStore::new(db.clone());
        store
            .ensure_table()
            .await
            .unwrap_or_else(|error| panic!("{label} ensure_table: {error}"));

        db.execute(&Stmt::new(
            "INSERT INTO tab_session_store (id, user_id, data, expiry) VALUES (?, ?, ?, ?)",
            vec![
                Value::from("session-1"),
                Value::from("user-1"),
                Value::from(r#"{"user_id":"user-1"}"#),
                Value::from(4_102_444_800_i64),
            ],
        ))
        .await
        .unwrap_or_else(|error| panic!("{label} insert: {error}"));

        let row = db
            .fetch_one(&Stmt::new(
                "SELECT user_id, data FROM tab_session_store WHERE id = ?",
                vec![Value::from("session-1")],
            ))
            .await
            .unwrap_or_else(|error| panic!("{label} select: {error}"));
        assert_eq!(row.opt_str("user_id").as_deref(), Some("user-1"), "{label}");

        let deleted = store
            .delete_user_sessions("user-1")
            .await
            .unwrap_or_else(|error| panic!("{label} delete_user_sessions: {error}"));
        assert_eq!(deleted, 1, "{label}");
        assert_eq!(
            store
                .count_user_sessions("user-1")
                .await
                .unwrap_or_else(|error| panic!("{label} count: {error}")),
            0,
            "{label}"
        );

        db.execute_script("DROP TABLE IF EXISTS tab_session_store")
            .await
            .ok();
    }
}
