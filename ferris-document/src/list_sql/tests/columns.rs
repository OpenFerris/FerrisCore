use super::*;

use crate::ListQuery;
use crate::engine::{DocumentEngine, EngineContext};
use ferris_schema::{DoctypeMeta, FieldMeta, IndexMeta};

fn field(name: &str, fieldtype: &str) -> FieldMeta {
    FieldMeta {
        fieldname: name.to_string(),
        fieldtype: fieldtype.to_string(),
        ..Default::default()
    }
}

/// The declared fields a probe `DocType` turns into real columns.
fn declared() -> Vec<FieldMeta> {
    vec![
        field("title", "Data"),
        field("qty", "Int"),
        field("active", "Check"),
        field("due", "Date"),
    ]
}

/// `qty > 5` over an `Int` column binds an `i64`, not the text `"5"`.
#[test]
fn an_int_range_is_pushed_with_an_integer_parameter() {
    let columns = declared();
    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("qty", FilterOp::Gt, "5")],
        None,
        None,
        &columns,
    );
    assert!(residual.is_empty(), "{residual:?}");
    assert!(plan.is_complete());
    let stmt = plan
        .select_page("tabListColumns", Dialect::SQLite, 20, 0)
        .unwrap();
    assert!(stmt.sql().contains("\"qty\" > ?"), "{}", stmt.sql());
    assert_eq!(stmt.params()[0], Value::from(5_i64));
}

/// Every comparison operator the deliverable names is pushed for a declared
/// numeric column, and `in` binds one typed value per part.
#[test]
fn numeric_in_and_every_range_operator_are_pushed() {
    let columns = declared();
    for (operator, sql_op) in [
        (FilterOp::Equals, "="),
        (FilterOp::Gt, ">"),
        (FilterOp::Gte, ">="),
        (FilterOp::Lt, "<"),
        (FilterOp::Lte, "<="),
    ] {
        let (plan, residual) = Plan::build(
            &HashMap::new(),
            &[op("qty", operator, "5")],
            None,
            None,
            &columns,
        );
        assert!(residual.is_empty(), "{operator:?}: {residual:?}");
        let sql = plan
            .select_all("tabListColumns", Dialect::SQLite)
            .unwrap()
            .sql()
            .to_string();
        assert!(
            sql.contains(&format!("\"qty\" {sql_op} ?")),
            "{operator:?}: {sql}"
        );
    }

    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("qty", FilterOp::In, "1, 2,3")],
        None,
        None,
        &columns,
    );
    assert!(residual.is_empty(), "{residual:?}");
    let stmt = plan.select_all("tabListColumns", Dialect::SQLite).unwrap();
    assert!(
        stmt.sql().contains("\"qty\" IN (?, ?, ?)"),
        "{}",
        stmt.sql()
    );
    assert_eq!(
        stmt.params(),
        &[Value::from(1_i64), Value::from(2_i64), Value::from(3_i64)]
    );
}

/// A text column keeps the equality and substring semantics the Rust
/// evaluator has, and the map spelling pushes the same predicate.
#[test]
fn text_equality_and_like_are_pushed() {
    let columns = declared();
    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("title", FilterOp::Equals, "x")],
        None,
        None,
        &columns,
    );
    assert!(residual.is_empty(), "{residual:?}");
    let stmt = plan.select_all("tabListColumns", Dialect::SQLite).unwrap();
    assert!(stmt.sql().contains("\"title\" = ?"), "{}", stmt.sql());
    assert_eq!(stmt.params()[0], Value::string("x"));

    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("title", FilterOp::Like, "ell")],
        None,
        None,
        &columns,
    );
    assert!(residual.is_empty(), "{residual:?}");
    let stmt = plan.select_all("tabListColumns", Dialect::SQLite).unwrap();
    assert!(
        stmt.sql().contains("LOWER(\"title\") LIKE LOWER(?)"),
        "{}",
        stmt.sql()
    );
    assert_eq!(stmt.params()[0], Value::string("%ell%"));

    let mut filters = HashMap::new();
    filters.insert("title".to_string(), Value::string("x"));
    let (plan, residual) = Plan::build(&filters, &[], None, None, &columns);
    assert!(residual.is_empty(), "{residual:?}");
    assert!(
        plan.count("tabListColumns", Dialect::SQLite)
            .unwrap()
            .sql()
            .contains("\"title\" = ?")
    );
}

/// A field the `DocType` did not turn into a column, a value that cannot be
/// coerced, and `""` equality all stay in Rust instead of producing a wrong
/// bound type.
#[test]
fn residual_cases_keep_the_plan_incomplete() {
    let columns = declared();

    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("tag", FilterOp::Equals, "keep")],
        None,
        None,
        &columns,
    );
    assert!(plan.filters_nothing());
    assert_eq!(residual[0].reason, ResidualReason::NotAColumn);
    assert!(!plan.is_complete());

    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("qty", FilterOp::Gt, "not a number")],
        None,
        None,
        &columns,
    );
    assert!(plan.filters_nothing(), "{residual:?}");
    assert_eq!(residual[0].reason, ResidualReason::ValueNotCoercible);
    assert!(!plan.is_complete());

    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("title", FilterOp::Equals, "")],
        None,
        None,
        &columns,
    );
    assert!(plan.filters_nothing(), "{residual:?}");
    assert_eq!(residual[0].reason, ResidualReason::EmptyStringNotPushed);
    assert!(!plan.is_complete());
}

/// A `Date` column stores validated date-only text, so a date equality is the
/// same comparison in SQL; a `Datetime`'s offset-bearing text is not, and its
/// precision-aware comparison stays in Rust.
#[test]
fn a_date_is_pushed_but_a_precision_aware_timestamp_is_not() {
    let columns = declared();
    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("due", FilterOp::Equals, "2026-01-02")],
        None,
        None,
        &columns,
    );
    assert!(residual.is_empty(), "{residual:?}");
    assert!(
        plan.select_all("tabListColumns", Dialect::SQLite)
            .unwrap()
            .sql()
            .contains("\"due\" = ?")
    );

    let with_datetime = vec![field("published_at", "Datetime")];
    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("published_at", FilterOp::Equals, "2026-10-30T12:34")],
        None,
        None,
        &with_datetime,
    );
    assert!(plan.filters_nothing(), "{residual:?}");
    assert_eq!(residual[0].reason, ResidualReason::TimestampNotPushed);
}

/// A declared column is a valid `ORDER BY` key, and the page still gets the
/// total-order tiebreaker.
#[test]
fn ordering_by_a_declared_column_is_sql() {
    let columns = declared();
    let (plan, residual) = Plan::build(&HashMap::new(), &[], None, Some("qty desc"), &columns);
    assert!(residual.is_empty(), "{residual:?}");
    assert!(!plan.orders_in_rust());
    let sql = plan
        .select_page("tabListColumns", Dialect::SQLite, 20, 0)
        .unwrap()
        .sql()
        .to_string();
    assert!(sql.contains("ORDER BY \"qty\" DESC"), "{sql}");
    assert!(sql.contains(", \"name\" ASC"), "{sql}");
}

fn register_probe() -> &'static str {
    const DOCTYPE: &str = "ListSqlColumnProbe";
    ferris_schema::register_doctype(DoctypeMeta {
        name: DOCTYPE.to_string(),
        fields: vec![field("title", "Data"), field("qty", "Int")],
        indexes: vec![IndexMeta {
            name: "idx_list_sql_column_probe_qty".to_string(),
            fields: vec!["qty".to_string()],
            unique: false,
            index_type: 0,
        }],
        ..Default::default()
    });
    DOCTYPE
}

async fn probe_engine(doctype: &str) -> (DocumentEngine, EngineContext<'static>) {
    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db);
    engine.ensure_table(doctype).await.unwrap();
    let ctx = EngineContext::new(doctype, "LSC-.#####").allow_anonymous(true);
    (engine, ctx)
}

fn qty(document: &Value) -> i64 {
    document
        .get("qty")
        .and_then(Value::as_i64)
        .unwrap_or_default()
}

/// End-to-end: the rows a pushed `qty > 5` returns are exactly the rows the
/// Rust evaluator would keep, and `COUNT(*)` counts the same set.
#[tokio::test]
async fn a_registered_int_column_filters_and_counts_in_sql() {
    let doctype = register_probe();
    let (engine, ctx) = probe_engine(doctype).await;
    for (title, quantity, tag) in [("x", 3_i64, "keep"), ("x", 6, "keep"), ("y", 9, "drop")] {
        engine
            .create(
                &ctx,
                ferris_core::value!({ "title": title, "qty": quantity, "tag": tag }),
            )
            .await
            .unwrap();
    }

    let mut params = ListQuery {
        filter_ops: vec![op("qty", FilterOp::Gt, "5")],
        ..Default::default()
    };
    let (documents, total) = engine.list(&ctx, &params).await.unwrap();
    assert_eq!(total, 2, "COUNT(*) frames the matching rows, not the page");
    let mut quantities: Vec<i64> = documents.iter().map(qty).collect();
    quantities.sort_unstable();
    assert_eq!(quantities, vec![6, 9]);

    // `title = "x"` over a real `Data` column returns only that title.
    params = ListQuery {
        filter_ops: vec![op("title", FilterOp::Equals, "x")],
        ..Default::default()
    };
    let (documents, total) = engine.list(&ctx, &params).await.unwrap();
    assert_eq!(total, 2);
    assert!(
        documents
            .iter()
            .all(|document| { document.get("title").and_then(Value::as_str) == Some("x") })
    );

    // Ordering by the declared column is a SQL sort (the column values are
    // numeric, so the answer is 9, 6, 3).
    params = ListQuery {
        order_by: Some("qty desc".to_string()),
        ..Default::default()
    };
    let (documents, total) = engine.list(&ctx, &params).await.unwrap();
    assert_eq!(total, 3);
    assert_eq!(documents.iter().map(qty).collect::<Vec<_>>(), vec![9, 6, 3]);
}

/// A residual filter makes the engine fall back to Rust, so `total` is the
/// number of rows that survived the *whole* filter — not the pushed subset.
#[tokio::test]
async fn a_residual_filter_keeps_the_total_honest() {
    let doctype = register_probe();
    let (engine, ctx) = probe_engine(doctype).await;
    for (title, quantity) in [("keep", 6_i64), ("drop", 9)] {
        engine
            .create(
                &ctx,
                ferris_core::value!({ "title": title, "qty": quantity }),
            )
            .await
            .unwrap();
    }

    let params = ListQuery {
        filter_ops: vec![op("qty", FilterOp::Gt, "5")],
        // A search term walks the document, so it has no column and stays in
        // Rust. Only one title contains it.
        search: Some("keep".to_string()),
        ..Default::default()
    };
    let (documents, total) = engine.list(&ctx, &params).await.unwrap();
    assert_eq!(documents.len(), 1, "{documents:?}");
    assert_eq!(
        total, 1,
        "the pushed subset has 2 rows; the Rust filter has 1"
    );
}

/// SQLite's own plan proves the predicate is served by the column index the
/// schema created for the declared field.
#[tokio::test]
async fn an_indexed_declared_column_is_used_by_sqlite() {
    let doctype = register_probe();
    let (engine, ctx) = probe_engine(doctype).await;
    for index in 0..30_i64 {
        let quantity = index % 10;
        engine
            .create(
                &ctx,
                ferris_core::value!({ "title": "row", "qty": quantity }),
            )
            .await
            .unwrap();
    }

    let columns = vec![field("title", "Data"), field("qty", "Int")];
    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("qty", FilterOp::Equals, "7")],
        None,
        None,
        &columns,
    );
    assert!(residual.is_empty(), "{residual:?}");
    let stmt = plan
        .select_page("tabListSqlColumnProbe", Dialect::SQLite, 20, 0)
        .unwrap();
    let explain = ferris_db::Stmt::new(
        format!("EXPLAIN QUERY PLAN {}", stmt.sql()),
        stmt.params().to_vec(),
    );
    let rows = engine.db().fetch_all(&explain).await.unwrap();
    let details: Vec<String> = rows
        .iter()
        .map(|row| row.opt_str("detail").unwrap_or_default())
        .collect();
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("USING INDEX")
                && detail.contains("list_sql_column_probe_qty")),
        "{details:?}"
    );
    assert!(
        !details.iter().any(|detail| detail.contains("SCAN")),
        "{details:?}"
    );
}
