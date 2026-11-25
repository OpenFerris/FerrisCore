//! Live PostgreSQL/MySQL check for typed columns and real child tables.
//!
//! The SQLite suite proves the engine; this proves the same write/read path on
//! the two production engines, including the `TEXT`/`DOUBLE`/`BOOLEAN` columns
//! and the child-table rows. It is skipped unless the corresponding
//! `FERRIS_TEST_{PG,MYSQL}_HOST` is set (CI sets both).

#![allow(unused_crate_dependencies)]

use ferris_core::{DatabaseConfig, DatabaseType, SslMode, Value};
use ferris_db::{Db, Stmt};
use ferris_document::{DocumentEngine, EngineContext};
use ferris_schema::{DoctypeMeta, FieldMeta};

fn table(db: &Db, name: &str) -> String {
    ferris_db::sql::Ident::new(name)
        .expect("valid table identifier")
        .quote(db.dialect())
}

fn field(name: &str, fieldtype: &str) -> FieldMeta {
    FieldMeta {
        fieldname: name.to_string(),
        fieldtype: fieldtype.to_string(),
        ..Default::default()
    }
}

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
async fn typed_columns_and_child_rows_round_trip_on_the_live_engines() {
    let configs = configs();
    if configs.is_empty() {
        eprintln!("skipping: no live database configured");
        return;
    }

    let parent = "TypedLiveParent";
    let child = "TypedLiveLine";
    ferris_schema::register_doctype(DoctypeMeta {
        name: child.to_string(),
        is_child_table: true,
        fields: vec![field("item", "Data"), field("amount", "Currency")],
        ..Default::default()
    });
    let mut lines = field("lines", "Table");
    lines.options = Some(child.to_string());
    ferris_schema::register_doctype(DoctypeMeta {
        name: parent.to_string(),
        fields: vec![
            field("title", "Data"),
            field("qty", "Int"),
            field("price", "Currency"),
            field("active", "Check"),
            field("due", "Date"),
            field("seen_at", "Datetime"),
            lines,
        ],
        ..Default::default()
    });

    for (label, config) in configs {
        let db = Db::connect(&config)
            .await
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        let parent_table = table(&db, "tabTypedLiveParent");
        let child_table = table(&db, "tabTypedLiveLine");
        db.execute_script(&format!("DROP TABLE IF EXISTS {child_table}"))
            .await
            .unwrap_or_else(|error| panic!("{label} drop child: {error}"));
        db.execute_script(&format!("DROP TABLE IF EXISTS {parent_table}"))
            .await
            .unwrap_or_else(|error| panic!("{label} drop parent: {error}"));

        let engine = DocumentEngine::new(db.clone());
        let ctx = EngineContext::new(parent, "TLV-.#####").allow_anonymous(true);
        let created = engine
            .create(
                &ctx,
                ferris_core::value!({
                    "title": "live",
                    "qty": 3,
                    "price": 12.5,
                    "active": true,
                    "due": "2026-01-02",
                    "seen_at": "2026-01-02T03:04:05Z",
                    "lines": [
                        {"item": "a", "amount": 1.5},
                        {"item": "b", "amount": 2.5}
                    ]
                }),
            )
            .await
            .unwrap_or_else(|error| panic!("{label} create: {error}"));
        let name = created
            .get("name")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();

        let row = db
            .fetch_one(&Stmt::new(
                &format!("SELECT * FROM {parent_table} WHERE name = ?"),
                vec![Value::from(name.clone())],
            ))
            .await
            .unwrap_or_else(|error| panic!("{label} select parent: {error}"));
        assert_eq!(row.opt_str("title").as_deref(), Some("live"), "{label}");
        assert_eq!(row.opt_i64("qty"), Some(3), "{label}");
        assert_eq!(row.opt_decimal_f64("price"), Some(12.5), "{label}");
        assert_eq!(row.opt_bool("active"), Some(true), "{label}");
        assert_eq!(
            row.opt_date("due").as_deref(),
            Some("2026-01-02"),
            "{label}"
        );

        let children = db
            .fetch_all(&Stmt::new(
                &format!("SELECT * FROM {child_table} WHERE parent = ? ORDER BY idx"),
                vec![Value::from(name.clone())],
            ))
            .await
            .unwrap_or_else(|error| panic!("{label} select children: {error}"));
        assert_eq!(children.len(), 2, "{label}");
        assert_eq!(children[0].opt_str("item").as_deref(), Some("a"), "{label}");
        assert_eq!(children[1].opt_str("item").as_deref(), Some("b"), "{label}");

        engine
            .update(
                &ctx,
                &name,
                ferris_core::value!({
                    "qty": 9,
                    "lines": [{"item": "c", "amount": 7.0}]
                }),
            )
            .await
            .unwrap_or_else(|error| panic!("{label} update: {error}"));
        let row = db
            .fetch_one(&Stmt::new(
                &format!("SELECT * FROM {parent_table} WHERE name = ?"),
                vec![Value::from(name.clone())],
            ))
            .await
            .unwrap_or_else(|error| panic!("{label} select update: {error}"));
        assert_eq!(row.opt_i64("qty"), Some(9), "{label}");
        let children = db
            .fetch_all(&Stmt::new(
                &format!("SELECT * FROM {child_table} WHERE parent = ? ORDER BY idx"),
                vec![Value::from(name.clone())],
            ))
            .await
            .unwrap_or_else(|error| panic!("{label} children after update: {error}"));
        assert_eq!(children.len(), 1, "{label}");
        assert_eq!(children[0].opt_str("item").as_deref(), Some("c"), "{label}");

        db.execute_script(&format!("DROP TABLE IF EXISTS {child_table}"))
            .await
            .ok();
        db.execute_script(&format!("DROP TABLE IF EXISTS {parent_table}"))
            .await
            .ok();
    }
}
