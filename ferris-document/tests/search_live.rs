//! Live PostgreSQL/MySQL check for the full-text search index.
//!
//! SQLite is covered by the unit test; this proves the same
//! create/search/update/delete cycle against the two production engines, each
//! with its own native index (`GIN` + `tsvector`, `FULLTEXT` + boolean mode).
//! It is skipped unless `FERRIS_TEST_{PG,MYSQL}_HOST` is set (CI sets both).
//!
//! The term is unique per run so a leftover row from an earlier, interrupted
//! run cannot turn a hit into a false positive.

#![allow(unused_crate_dependencies)]

use ferris_core::{DatabaseConfig, DatabaseType, SslMode, Value};
use ferris_db::Db;
use ferris_document::{DocumentEngine, EngineContext, ListQuery};
use ferris_schema::{DoctypeMeta, FieldMeta};

const DOCTYPE: &str = "SearchLiveProbe";

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

/// The names `list` returns for `term`, ordered so assertions are stable.
async fn hits(engine: &DocumentEngine, ctx: &EngineContext<'_>, term: &str) -> Vec<String> {
    let params = ListQuery {
        search: Some(term.to_string()),
        ..Default::default()
    };
    let (documents, total) = engine
        .list(ctx, &params)
        .await
        .unwrap_or_else(|error| panic!("list search `{term}`: {error}"));
    assert_eq!(
        documents.len() as u64,
        total,
        "the page and the total disagree for `{term}`"
    );
    let mut names: Vec<String> = documents
        .iter()
        .filter_map(|document| {
            document
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn full_text_search_round_trips_on_the_live_engines() {
    let configs = configs();
    if configs.is_empty() {
        eprintln!("skipping: no live database configured");
        return;
    }
    ferris_schema::register_doctype(DoctypeMeta {
        name: DOCTYPE.to_string(),
        fields: vec![FieldMeta {
            fieldname: "title".to_string(),
            fieldtype: "Data".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    });

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let term = format!("needle{unique}");
    // `haystack` is the replacement word, distinct from the unique term.
    let fresh = format!("{term} alpha");

    for (label, config) in configs {
        let db = Db::connect(&config)
            .await
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        let engine = DocumentEngine::new(db);
        let ctx = EngineContext::new(DOCTYPE, "").allow_anonymous(true);

        let created = engine
            .create(
                &ctx,
                Value::from_json(serde_json::json!({ "title": fresh })),
            )
            .await
            .unwrap_or_else(|error| panic!("{label} create: {error}"));
        let name = created
            .get("name")
            .and_then(Value::as_str)
            .expect("created document has a name")
            .to_string();

        assert_eq!(
            hits(&engine, &ctx, &term).await,
            vec![name.clone()],
            "{label}: the native index must find the new document"
        );

        // An update replaces the indexed body.
        engine
            .update(
                &ctx,
                &name,
                Value::from_json(serde_json::json!({ "title": "haystack only" })),
            )
            .await
            .unwrap_or_else(|error| panic!("{label} update: {error}"));
        assert!(
            hits(&engine, &ctx, &term).await.is_empty(),
            "{label}: the old token must be gone after the update"
        );
        assert_eq!(
            hits(&engine, &ctx, "haystack").await,
            vec![name.clone()],
            "{label}: the new token must be found"
        );

        // A delete removes the name from the index.
        engine
            .delete(&ctx, &name)
            .await
            .unwrap_or_else(|error| panic!("{label} delete: {error}"));
        assert!(
            hits(&engine, &ctx, "haystack").await.is_empty(),
            "{label}: the deleted document must not be found"
        );
    }
}
