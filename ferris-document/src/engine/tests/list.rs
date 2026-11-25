use super::*;

#[tokio::test]
async fn test_list_filters_and_total() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);
    engine
        .create(
            &ctx,
            ferris_core::value!({ "title": "a", "status": "open" }),
        )
        .await
        .unwrap();
    engine
        .create(
            &ctx,
            ferris_core::value!({ "title": "b", "status": "closed" }),
        )
        .await
        .unwrap();
    engine
        .create(
            &ctx,
            ferris_core::value!({ "title": "c", "status": "open" }),
        )
        .await
        .unwrap();

    let mut params = crate::ListQuery::default();
    params
        .filters
        .insert("status".to_string(), Value::from("open"));
    let (documents, total) = engine.list(&ctx, &params).await.unwrap();
    assert_eq!(total, 2);
    assert_eq!(documents.len(), 2);
}

#[tokio::test]
async fn test_list_restricted_to_a_set_of_names() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);
    engine
        .create(&ctx, ferris_core::value!({ "title": "a" }))
        .await
        .unwrap();
    engine
        .create(&ctx, ferris_core::value!({ "title": "b" }))
        .await
        .unwrap();
    engine
        .create(&ctx, ferris_core::value!({ "title": "c" }))
        .await
        .unwrap();

    // Take a real name back so the test does not depend on the naming series.
    let (_, total) = engine
        .list(&ctx, &crate::ListQuery::default())
        .await
        .unwrap();
    assert_eq!(total, 3);
    let (one, _) = engine
        .list(
            &ctx,
            &crate::ListQuery {
                limit: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let name = one[0]
        .get("name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();

    // The total follows the restriction, so pagination stays truthful.
    let params = crate::ListQuery {
        only_names: Some(HashSet::from([name.clone()])),
        ..Default::default()
    };
    let (documents, total) = engine.list(&ctx, &params).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(
        documents[0].get("name").and_then(Value::as_str),
        Some(name.as_str())
    );

    // An empty set matches nothing rather than everything.
    let none = crate::ListQuery {
        only_names: Some(HashSet::new()),
        ..Default::default()
    };
    let (_, total) = engine.list(&ctx, &none).await.unwrap();
    assert_eq!(total, 0);
}

// ---------------------------------------------------------------------
// §C2 — the second half: the list must be a *query*, not a table scan
//
// The tests above pin the answers. These pin the query, because the answers
// are identical before and after the push-down and only the SQL text
// distinguishes them.
//
// They read the statements the engine builds rather than run them, which is
// deliberate: the point is *which* SQL is emitted, and a behavioural test
// cannot tell an indexed `WHERE`+`LIMIT` from a full scan that happened to
// return the right rows. `plan` is a pure function, so this is possible
// here in a way it is not for the engine's callers.
// ---------------------------------------------------------------------

/// Every column-backed filter has to reach SQL, with the value bound. This
/// is the property that makes the push-down worth anything: a `WHERE` that
/// drops rows in the database is one an index can serve.
#[test]
fn a_column_filter_becomes_a_bound_where_term() {
    let mut params = crate::ListQuery::default();
    params
        .filters
        .insert("owner".to_string(), Value::string("alice@local"));
    let (plan, residual) = list_sql::Plan::build(
        &params.filters,
        &params.filter_ops,
        params.search.as_deref(),
        params.order_by.as_deref(),
        &[],
    );
    assert!(residual.is_empty(), "{residual:?}");
    let sql = plan
        .select_all("tabToDo", Dialect::SQLite)
        .unwrap()
        .sql()
        .to_string();
    assert!(sql.contains("WHERE"), "{sql}");
    assert!(sql.contains("\"owner\" = ?"), "{sql}");
    // The value is bound, never spliced: `alice@local` is not in the text.
    assert!(!sql.contains("alice@local"), "{sql}");
}

/// The one place where the push-down is not merely faster but *different*,
/// and therefore observable: `docstatus` is a `SMALLINT` column holding
/// `0/1/2` while the document holds the words `draft`/`submitted`/`cancelled`.
///
/// Ordering by it in SQL yields the real lifecycle order (cancelled, then
/// submitted, then draft). Sorting the words in Rust yields alphabetical
/// order, which puts `submitted` before `draft` — the opposite of the
/// lifecycle, and a list a user would read as wrong. So this test both pins
/// the fix and shows what the fallback actually costs: not just speed.
#[tokio::test]
async fn ordering_by_docstatus_follows_the_lifecycle_not_the_alphabet() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);
    let submitted = engine
        .create(&ctx, ferris_core::value!({ "title": "s" }))
        .await
        .unwrap();
    let cancelled = engine
        .create(&ctx, ferris_core::value!({ "title": "c" }))
        .await
        .unwrap();
    let _draft = engine
        .create(&ctx, ferris_core::value!({ "title": "d" }))
        .await
        .unwrap();
    let name = |v: &Value| {
        v.get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_string()
    };
    for (doc, status) in [
        (&submitted, DocStatus::Submitted),
        (&cancelled, DocStatus::Submitted),
    ] {
        engine
            .set_docstatus(&ctx, &name(doc), status)
            .await
            .unwrap();
    }
    // `Draft -> Cancelled` is not a legal transition, so cancel from
    // `Submitted` as the workflow requires.
    engine
        .set_docstatus(&ctx, &name(&cancelled), DocStatus::Cancelled)
        .await
        .unwrap();

    let (docs, total) = engine
        .list(
            &ctx,
            &crate::ListQuery {
                order_by: Some("docstatus desc".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(total, 3);
    let order: Vec<&str> = docs
        .iter()
        .map(|d| {
            d.get("docstatus")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        order,
        vec!["cancelled", "submitted", "draft"],
        "docstatus must order by its numeric value, not as words"
    );
}

/// A `LIMIT` in the emitted SQL is the whole point of the change: the old
/// `list` had none and materialized every row before paginating in Rust.
#[test]
fn a_list_request_carries_a_limit_into_sql() {
    let (plan, _) = list_sql::Plan::build(&HashMap::new(), &[], None, None, &[]);
    let stmt = plan
        .select_page("tabToDo", Dialect::SQLite, 20, 40)
        .unwrap();
    let sql = stmt.sql();
    assert!(sql.contains("LIMIT ? OFFSET ?"), "{sql}");
    assert_eq!(
        plan.residual_count(),
        0,
        "an unfiltered request has nothing to leave in Rust"
    );
}

/// A filter the database cannot answer must not be silently dropped, and it
/// must not be turned into SQL against a column that does not exist — the
/// row would be lost with no error.
#[test]
fn a_blob_filter_is_reported_and_not_spliced_into_sql() {
    let mut params = crate::ListQuery::default();
    params
        .filters
        .insert("status".to_string(), Value::string("open"));
    let (plan, residual) = list_sql::Plan::build(
        &params.filters,
        &params.filter_ops,
        params.search.as_deref(),
        params.order_by.as_deref(),
        &[],
    );
    assert_eq!(residual.len(), 1, "{residual:?}");
    assert_eq!(residual[0].field, "status");
    let sql = plan
        .select_all("tabToDo", Dialect::SQLite)
        .unwrap()
        .sql()
        .to_string();
    assert!(!sql.contains("\"status\""), "{sql}");
    assert!(!sql.contains("WHERE"), "{sql}");
    assert!(
        !plan.is_complete(),
        "with a filter left in Rust, SQL may not own the page or the total"
    );
}

/// `only_names` is how the API layer hands the engine a decision it could
/// not express as a document filter (see `only_liked` in `ferris-api`). It is
/// a `name` predicate, and `name` is a column.
#[test]
fn a_name_set_becomes_an_in_predicate_in_sql() {
    let names: HashSet<String> = ["TODO-0001".to_string(), "TODO-0002".to_string()].into();
    let plan = list_sql::Plan::default()
        .with_in("name", names.iter().cloned().collect())
        .unwrap();
    let sql = plan
        .select_all("tabToDo", Dialect::Postgres)
        .unwrap()
        .sql()
        .to_string();
    assert!(sql.contains("\"name\" IN (?, ?)"), "{sql}");
    assert!(!sql.contains("TODO-0001"), "{sql}");
}

/// The tiebreaker is what makes a page stable. Two identical requests over
/// unchanged data must return the same rows, and they can only do that if
/// the database is told to produce a total order.
///
/// The final key has to be the `name` **column**: using the table
/// identifier (the bug this asserts against) is rejected by PostgreSQL and
/// collapses to a constant on SQLite/MySQL, so the order stays partial.
#[test]
fn paging_is_stable_across_identical_requests() {
    let (plan, _) = list_sql::Plan::build(&HashMap::new(), &[], None, Some("modified desc"), &[]);
    for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
        let first = plan.select_page("tabToDo", dialect, 5, 10).unwrap();
        let second = plan.select_page("tabToDo", dialect, 5, 10).unwrap();
        assert_eq!(first.sql(), second.sql(), "{dialect:?}");
        assert!(
            first.sql().contains("ORDER BY"),
            "{dialect:?}: {}",
            first.sql()
        );
        // `name` is appended as the final sort key on every path — and it
        // names the column, not the table. MySQL quotes identifiers with
        // backticks, the others with double quotes.
        let tiebreaker = match dialect {
            Dialect::MySQL => "`name` ASC",
            Dialect::Postgres | Dialect::SQLite => "\"name\" ASC",
        };
        assert!(
            first.sql().contains(&format!(", {tiebreaker}")),
            "{dialect:?}: {}",
            first.sql()
        );
    }
}

// ---------------------------------------------------------------------
// R1 — lazy JSON index materialization and index-served lists
// ---------------------------------------------------------------------

/// The end-to-end R1 promise: a registered `DocType` with a single-field
/// index over a blob field gets that index materialized lazily by
/// `ensure_table` (idempotently), and an exact `eq` on that field is served
/// by the index — `EXPLAIN QUERY PLAN` says `USING INDEX`, never `SCAN` —
/// while the count stays honest because the plan is complete.
#[tokio::test]
async fn r1_json_index_is_materialized_and_serves_the_list() {
    let doctype = "SolmecReadR1";
    // A `device_id`-style blob field, declared indexed through the same
    // `meta.indexes` entry the SolmecBOX DocTypes use. Registered globally
    // because `ensure_table` and `list` both read the in-process registry
    // for a context that carries no metadata.
    ferris_schema::register_doctype(ferris_schema::DoctypeMeta {
        name: doctype.to_string(),
        fields: vec![
            ferris_schema::FieldMeta::data("device_id"),
            ferris_schema::FieldMeta::int("reading"),
        ],
        indexes: vec![ferris_schema::IndexMeta {
            name: "idx_reads_device".to_string(),
            fields: vec!["device_id".to_string()],
            unique: false,
            index_type: 0,
        }],
        ..Default::default()
    });

    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db.clone());
    let table = engine.ensure_table(doctype).await.unwrap();
    assert_eq!(table, "tabSolmecReadR1");

    // A second engine over the same database re-runs the whole DDL against
    // an existing table, columns and index. The ensures tolerate the
    // duplicate errors, so this is a no-op and the index exists exactly
    // once — the boot-after-migrate case. `device_id` is a declared field, so
    // the index is over its real column.
    DocumentEngine::new(db.clone())
        .ensure_table(doctype)
        .await
        .unwrap();
    let indexes = db
        .fetch_one(&Stmt::new(
            "SELECT COUNT(*) AS n FROM sqlite_master WHERE type = 'index' AND name = ?",
            vec![Value::from("idx_tabSolmecReadR1_reads_device")],
        ))
        .await
        .unwrap();
    assert_eq!(
        indexes.count("n"),
        1,
        "the declared index was created once on the column"
    );

    let ctx = EngineContext::new(doctype, "SOLR-.#####").allow_anonymous(true);
    for device in ["DEV-0001", "DEV-0001", "DEV-0002"] {
        engine
            .create(
                &ctx,
                ferris_core::value!({ "device_id": device, "reading": 1 }),
            )
            .await
            .unwrap();
    }

    let mut params = crate::ListQuery::default();
    params
        .filters
        .insert("device_id".to_string(), Value::string("DEV-0001"));
    let (documents, total) = engine.list(&ctx, &params).await.unwrap();
    assert_eq!(documents.len(), 2, "{documents:?}");
    assert_eq!(
        total, 2,
        "count frames every matching row, not just this page"
    );

    // Rebuild the exact statement the engine issued and ask the database to
    // justify it. A residual would leave the plan incomplete and SQL would
    // not be trusted for the page at all, so `is_complete` has to hold for
    // this check to mean anything.
    let columns: Vec<ferris_schema::FieldMeta> = ferris_schema::get_doctype(doctype)
        .as_deref()
        .unwrap()
        .fields
        .iter()
        .filter(|field| {
            ferris_schema::doctype_columns::field_column(field, db.dialect())
                .ok()
                .flatten()
                .is_some()
        })
        .cloned()
        .collect();
    let plan = crate::list_sql::Plan::build(
        &params.filters,
        &params.filter_ops,
        params.search.as_deref(),
        params.order_by.as_deref(),
        &columns,
    )
    .0;
    assert!(
        plan.is_complete(),
        "an exact eq over an indexed field pushes"
    );
    let stmt = plan
        .select_page(
            &table,
            db.dialect(),
            params.limit.unwrap_or(20),
            params.offset.unwrap_or(0),
        )
        .unwrap();
    let explain = Stmt::new(
        format!("EXPLAIN QUERY PLAN {}", stmt.sql()),
        stmt.params().to_vec(),
    );
    let rows = db.fetch_all(&explain).await.unwrap();
    let details: Vec<String> = rows
        .iter()
        .map(|row| row.opt_str("detail").unwrap_or_default())
        .collect();
    assert!(
        details
            .iter()
            .any(|d| d.contains("USING INDEX idx_tabSolmecReadR1_reads_device")),
        "the eq must be served by the JSON index: {details:?}"
    );
    assert!(
        !details.iter().any(|d| d.contains("SCAN")),
        "an indexed field must not trigger a full scan: {details:?}"
    );
}

#[tokio::test]
async fn list_hides_documents_the_caller_cannot_read() {
    let engine = engine().await;
    let admin = sysadmin();
    let alice = plain_user();
    let admin_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&admin);
    let alice_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&alice);

    // Two documents owned by Alice and one by the admin. Created through
    // `create_owned_by` because Alice has no installed policy granting her
    // `Create` (§C3) — the only principal that can create anything today is
    // a `System Manager`.
    for title in ["mine one", "mine two"] {
        engine
            .create_owned_by(
                &admin_ctx,
                "alice@local",
                ferris_core::value!({ "title": title }),
            )
            .await
            .unwrap();
    }
    engine
        .create(&admin_ctx, ferris_core::value!({ "title": "admin's" }))
        .await
        .unwrap();

    // Alice sees only her own two, and `total` agrees with the page.
    let (docs, total) = engine
        .list(&alice_ctx, &crate::ListQuery::default())
        .await
        .unwrap();
    assert_eq!(total, 2, "total must count only what the caller may read");
    assert_eq!(docs.len(), 2);
    for doc in &docs {
        assert_eq!(
            doc.get("owner").and_then(|v| v.as_str()),
            Some("alice@local"),
            "a row alice cannot read leaked into her list"
        );
    }

    // Admin sees all three.
    let (_, total) = engine
        .list(&admin_ctx, &crate::ListQuery::default())
        .await
        .unwrap();
    assert_eq!(total, 3);

    // Pagination applies *after* the filter, so a page is never short
    // because the rows behind it were dropped.
    let page = crate::ListQuery {
        limit: Some(1),
        ..Default::default()
    };
    let (docs, total) = engine.list(&alice_ctx, &page).await.unwrap();
    assert_eq!((docs.len(), total), (1, 2));
}
