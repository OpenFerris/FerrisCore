use super::*;
use std::collections::HashSet;

/// A `DocType` with metadata, because the search index is ensured by
/// `ensure_table` only for `DocType`s the registry knows: an ad-hoc test name
/// would keep the Rust scan and prove nothing.
const DOCTYPE: &str = "FtsSearchProbe";

fn register() {
    ferris_schema::register_doctype(DoctypeMeta {
        name: DOCTYPE.to_string(),
        fields: vec![field("title", "Data", 0)],
        ..Default::default()
    });
}

async fn create(engine: &DocumentEngine, ctx: &EngineContext<'_>, title: &str) -> String {
    let document = engine
        .create(ctx, ferris_core::value!({ "title": title }))
        .await
        .unwrap();
    document
        .get("name")
        .and_then(Value::as_str)
        .expect("created document has a name")
        .to_string()
}

async fn search_names(engine: &DocumentEngine, term: &str) -> HashSet<String> {
    crate::engine::search::names(engine.db(), DOCTYPE, term)
        .await
        .unwrap()
}

/// The end-to-end contract of the index: writes populate it, an update moves
/// the hit, a delete removes it, and `list` uses it instead of the blob scan.
///
/// A build without FTS5 takes the other branch: `ensure` fails, and the same
/// `list` search must still answer, through the Rust scan.
#[tokio::test]
async fn full_text_index_answers_search_and_tracks_writes() {
    register();
    let engine = engine().await;
    let ctx = EngineContext::new(DOCTYPE, "").allow_anonymous(true);
    // Exercises the integration point: metadata exists, so `ensure_table` runs
    // the search DDL itself.
    engine.ensure_table(DOCTYPE).await.unwrap();

    if let Err(error) = crate::engine::search::ensure(engine.db()).await {
        eprintln!("FTS5 unavailable ({error}); asserting the Rust scan fallback");
        assert!(!crate::engine::search::ready(engine.db()));
        let name = create(&engine, &ctx, "needle in a haystack").await;
        let params = crate::ListQuery {
            search: Some("needle".to_string()),
            ..Default::default()
        };
        let (documents, total) = engine.list(&ctx, &params).await.unwrap();
        assert_eq!(total, 1, "the scan fallback must still find the document");
        assert_eq!(
            documents[0].get("name").and_then(Value::as_str),
            Some(name.as_str())
        );
        return;
    }
    assert!(crate::engine::search::ready(engine.db()));

    let alpha = create(&engine, &ctx, "needle alpha").await;
    let beta = create(&engine, &ctx, "nothing here").await;
    assert_eq!(
        search_names(&engine, "needle").await,
        HashSet::from([alpha.clone()]),
        "only the document containing the token is indexed"
    );

    // Proof that `list` no longer scans in Rust: a row written around the
    // engine is absent from the index, so the scan would find it and the index
    // path must not. `name` and `title` are the only columns that have to be
    // right.
    let table = ferris_schema::migration::document_table(DOCTYPE).unwrap();
    engine
        .db()
        .execute(&Stmt::new(
            format!(
                "INSERT INTO {table} \
                 (name, owner, docstatus, creation, modified, title) \
                 VALUES (?, ?, ?, ?, ?, ?)"
            ),
            vec![
                Value::from("unindexed"),
                Value::Null,
                Value::from(0_i64),
                Value::from(1_i64),
                Value::from(1_i64),
                Value::from("needle phantom"),
            ],
        ))
        .await
        .unwrap();

    let params = crate::ListQuery {
        search: Some("needle".to_string()),
        ..Default::default()
    };
    let (documents, total) = engine.list(&ctx, &params).await.unwrap();
    assert_eq!(total, 1, "the index answered; the Rust scan did not run");

    assert_eq!(
        documents[0].get("name").and_then(Value::as_str),
        Some(alpha.as_str())
    );

    // Intersection with an existing `only_names`: the search may only narrow.
    let params = crate::ListQuery {
        search: Some("needle".to_string()),
        only_names: Some(HashSet::from([beta.clone()])),
        ..Default::default()
    };
    let (documents, total) = engine.list(&ctx, &params).await.unwrap();
    assert_eq!(total, 0);
    assert!(documents.is_empty());

    let params = crate::ListQuery {
        search: Some("needle".to_string()),
        only_names: Some(HashSet::from([alpha.clone()])),
        ..Default::default()
    };
    let (_, total) = engine.list(&ctx, &params).await.unwrap();
    assert_eq!(total, 1);

    // An update replaces the indexed body instead of appending to it.
    engine
        .update(
            &ctx,
            &alpha,
            ferris_core::value!({ "title": "haystack only" }),
        )
        .await
        .unwrap();
    assert!(
        search_names(&engine, "needle").await.is_empty(),
        "the old token must be gone after the update"
    );
    assert_eq!(
        search_names(&engine, "haystack").await,
        HashSet::from([alpha.clone()])
    );

    // A delete takes the name out of the index.
    engine.delete(&ctx, &alpha).await.unwrap();
    assert!(search_names(&engine, "haystack").await.is_empty());

    // Token semantics, not substring: `alphabet` is a different token.
    create(&engine, &ctx, "alphabet soup").await;
    assert!(
        search_names(&engine, "alpha").await.is_empty(),
        "the index matches whole tokens, not substrings"
    );

    // `ferris migrate` calls this after the schema backfill: a row written
    // around the engine becomes searchable without a rewrite through it.
    let rebuilt = engine.reindex_search(DOCTYPE).await.unwrap();
    assert!(rebuilt >= 1, "reindexed {rebuilt} rows");
    let names = search_names(&engine, "needle").await;
    assert!(names.contains("unindexed"), "{names:?}");
}
