use super::*;

// ---------------------------------------------------------------------
// R1 — JSON expression pushdown for indexed blob fields
// ---------------------------------------------------------------------

/// The new pushdown: a DocType-declared index over a blob field turns an
/// exact `eq` into a SQL equality against the same expression the index
/// was created over (`json_field_target`), on every dialect.
#[test]
fn an_indexed_blob_field_pushes_an_exact_eq_as_a_json_expression() {
    let indexed = ["device_id".to_string()];
    let (plan, residual) = Plan::build(
        &filters(&[("device_id", "DEV-0001")]),
        &[],
        None,
        None,
        &indexed,
    );
    assert!(residual.is_empty(), "{residual:?}");
    assert!(!plan.filters_nothing());
    assert!(plan.is_complete(), "the index made the predicate pushable");

    for (dialect, target) in [
        (Dialect::SQLite, "json_extract(\"data\", '$.device_id')"),
        (Dialect::Postgres, "(\"data\"::jsonb ->> 'device_id')"),
        (Dialect::MySQL, "`g_device_id`"),
    ] {
        let sql = plan
            .select_all("tabSolmecDevice", dialect)
            .unwrap()
            .sql()
            .to_string();
        assert!(sql.contains(&format!("{target} = ?")), "{dialect:?}: {sql}");
        // The value is bound, never spliced.
        assert!(!sql.contains("DEV-0001"), "{dialect:?}: {sql}");
    }
}

/// A field that lives in the blob but is *not* declared indexed stays
/// residual with the same `NotAColumn` reason as before R1 — the index
/// declaration, not the field's location, is what authorizes pushdown.
#[test]
fn an_indexed_field_stays_residual_without_the_index_declaration() {
    let (plan, residual) =
        Plan::build(&filters(&[("device_id", "DEV-0001")]), &[], None, None, &[]);
    assert_eq!(residual.len(), 1, "{residual:?}");
    assert_eq!(residual[0].field, "device_id");
    assert_eq!(residual[0].reason, ResidualReason::NotAColumn);
    assert!(
        !plan.is_complete(),
        "without the index, SQL cannot own the count"
    );
}

/// `in` over an indexed blob field becomes an `IN` over the JSON
/// expression, with one placeholder per value and the same empty-list
/// contradiction the column path uses.
#[test]
fn an_indexed_blob_field_pushes_in_as_a_json_expression() {
    let indexed = ["status".to_string()];
    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("status", FilterOp::In, "open, closed")],
        None,
        None,
        &indexed,
    );
    assert!(residual.is_empty(), "{residual:?}");
    let sql = plan
        .select_all("tabSolmecDevice", Dialect::SQLite)
        .unwrap()
        .sql()
        .to_string();
    assert!(
        sql.contains("json_extract(\"data\", '$.status') IN (?, ?)"),
        "{sql}"
    );

    // An `in` with only empty parts stays in Rust: `cell_str` reads a
    // missing cell as `""`, so the Rust evaluator matches rows *without*
    // the field, which a SQL `IN` (or the `1 = 0` contradiction an empty
    // list would render) never can.
    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("status", FilterOp::In, " , , ")],
        None,
        None,
        &indexed,
    );
    assert!(
        plan.filters_nothing(),
        "an all-empty in list is not pushable"
    );
    assert_eq!(residual.len(), 1, "{residual:?}");
    assert_eq!(residual[0].reason, ResidualReason::JsonExactNotPushed);
    assert!(!plan.is_complete(), "the count belongs to the Rust path");
}

/// Ranges and `LIKE` over an indexed blob field deliberately stay in Rust:
/// the Rust comparator is numeric-when-both-parse and case-insensitive, a
/// rule SQL text comparison does not satisfy, and the exact-text index
/// could not serve them anyway. Each is reported with its own reason.
#[test]
fn ranges_and_like_over_an_indexed_blob_field_stay_residual() {
    let indexed = ["ts".to_string()];
    for (opcode, reason) in [
        (FilterOp::Gt, ResidualReason::JsonRangeNotPushed),
        (FilterOp::Gte, ResidualReason::JsonRangeNotPushed),
        (FilterOp::Lt, ResidualReason::JsonRangeNotPushed),
        (FilterOp::Lte, ResidualReason::JsonRangeNotPushed),
        (FilterOp::Like, ResidualReason::JsonLikeNotPushed),
    ] {
        let (plan, residual) = Plan::build(
            &HashMap::new(),
            &[op("ts", opcode, "2026-01-01")],
            None,
            None,
            &indexed,
        );
        assert!(plan.filters_nothing(), "{opcode:?}");
        assert_eq!(residual.len(), 1, "{opcode:?}: {residual:?}");
        assert_eq!(residual[0].reason, reason, "{opcode:?}");
    }
}

/// Ordering by an indexed blob field becomes a SQL sort over the JSON
/// expression — with missing/null values last in *both* directions, the
/// same rule `filters::order_documents` applies — and the page stays
/// total-ordered by the `name` tiebreaker.
#[test]
fn ordering_by_an_indexed_blob_field_is_served_by_sql() {
    let indexed = ["ts".to_string()];
    let (plan, residual) = Plan::build(&HashMap::new(), &[], None, Some("ts desc"), &indexed);
    assert!(residual.is_empty(), "{residual:?}");
    assert!(!plan.orders_in_rust());
    let sql = plan
        .select_page("tabSolmecRead", Dialect::SQLite, 20, 0)
        .unwrap()
        .sql()
        .to_string();
    assert!(sql.contains("json_extract(\"data\", '$.ts')"), "{sql}");
    assert!(sql.contains("IS NULL ASC"), "{sql}");
    assert!(sql.contains("DESC"), "{sql}");
    assert!(sql.contains(", \"name\" ASC"), "{sql}");
}

/// A `COUNT(*)` over an indexed field counts the pushed-down subset and is
/// honest by construction: the plan is complete, so «the subset» *is* the
/// row set the filter asked for.
#[test]
fn count_over_an_indexed_blob_field_is_complete() {
    let indexed = ["device_id".to_string()];
    let (plan, residual) = Plan::build(
        &filters(&[("device_id", "DEV-0001")]),
        &[],
        None,
        None,
        &indexed,
    );
    assert!(residual.is_empty());
    assert!(plan.is_complete());
    let sql = plan
        .count("tabSolmecDevice", Dialect::SQLite)
        .unwrap()
        .sql()
        .to_string();
    assert!(sql.contains("COUNT(*)"), "{sql}");
    assert!(
        sql.contains("json_extract(\"data\", '$.device_id') = ?"),
        "{sql}"
    );
}

/// The exact-equality pushdown must hold for a hash-map filter too, not
/// only for the structural operator list — both are legal `ListQuery`
/// spellings of the same filter.
#[test]
fn a_hash_map_eq_over_an_indexed_field_pushes_the_same_way() {
    let indexed = ["serial".to_string()];
    let (plan, residual) = Plan::build(&filters(&[("serial", "SN-9")]), &[], None, None, &indexed);
    assert!(residual.is_empty(), "{residual:?}");
    let sql = plan
        .select_all("tabSolmecSensor", Dialect::SQLite)
        .unwrap()
        .sql()
        .to_string();
    assert!(
        sql.contains("json_extract(\"data\", '$.serial') = ?"),
        "{sql}"
    );
}

/// A field that does not exist anywhere must not reach SQL even when the
/// caller names it as indexed. A declared-but-invalid identifier cannot be
/// rendered (`json_field_target` validates it) and fails loudly, never as
/// SQL against a column the database would reject.
#[test]
fn a_declared_index_field_with_an_invalid_name_fails_loudly() {
    let indexed = ["bad name".to_string()];
    let (plan, residual) = Plan::build(&filters(&[("bad name", "x")]), &[], None, None, &indexed);
    assert!(residual.is_empty(), "{residual:?}");
    assert!(plan.select_all("tabX", Dialect::SQLite).is_err());
}

// --- the exact-value guard ---------------------------------------------

/// `eq ""` is only honest in SQL for rows that literally carry `""`.
/// `cell_str` reads a *missing* cell as `""`, so the Rust filter also
/// matches rows without the field, and `json_extract(...) = ''` never
/// matches a SQL NULL. Both spellings of the filter stay in Rust.
#[test]
fn an_empty_eq_over_an_indexed_json_field_stays_residual() {
    let indexed = ["device_id".to_string()];

    // The structural operator spelling.
    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("device_id", FilterOp::Equals, "")],
        None,
        None,
        &indexed,
    );
    assert!(plan.filters_nothing(), "{residual:?}");
    assert_eq!(residual.len(), 1, "{residual:?}");
    assert_eq!(residual[0].reason, ResidualReason::JsonExactNotPushed);
    assert_eq!(residual[0].field, "device_id");
    assert!(!plan.is_complete(), "the count belongs to the Rust path");

    // The hash-map spelling behaves identically.
    let (plan, residual) = Plan::build(&filters(&[("device_id", "")]), &[], None, None, &indexed);
    assert!(plan.filters_nothing(), "{residual:?}");
    assert_eq!(residual.len(), 1, "{residual:?}");
    assert_eq!(residual[0].reason, ResidualReason::JsonExactNotPushed);
}

/// The same guard for a timestamp: the evaluator compares a filter at the
/// precision the needle names (`equals 12:34` matches `12:34:56`), while the
/// exact-text JSON index would only match the literal `12:34`. Pushing it
/// would make the SQL path return fewer rows than the Rust path.
#[test]
fn a_timestamp_eq_over_an_indexed_json_field_stays_residual() {
    let indexed = ["published_at".to_string()];
    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("published_at", FilterOp::Equals, "2026-10-30T12:34")],
        None,
        None,
        &indexed,
    );
    assert!(plan.filters_nothing(), "{residual:?}");
    assert_eq!(residual.len(), 1, "{residual:?}");
    assert_eq!(residual[0].reason, ResidualReason::JsonExactNotPushed);
    assert!(!plan.is_complete(), "the count belongs to the Rust path");
}

/// The same coercion breaks `in` on *one* empty part: `a,` means "a or a
/// missing cell" in Rust (`cell_str` reads the missing cell as `""`), and
/// no SQL `IN` can express the second branch. Keeping the whole filter in
/// Rust is what makes the empty case in the `in` test above a guard too,
/// not just a special case of "no values".
#[test]
fn an_in_with_an_empty_part_stays_residual() {
    let indexed = ["status".to_string()];
    for raw in ["a,", ",a", "a, b,"] {
        let (plan, residual) = Plan::build(
            &HashMap::new(),
            &[op("status", FilterOp::In, raw)],
            None,
            None,
            &indexed,
        );
        assert!(plan.filters_nothing(), "{raw}: {residual:?}");
        assert_eq!(residual.len(), 1, "{raw}: {residual:?}");
        assert_eq!(
            residual[0].reason,
            ResidualReason::JsonExactNotPushed,
            "{raw}"
        );
    }
}

/// The hash-map spelling carries a typed `Value`, and only a string is
/// ever pushed: a number compares structurally in Rust (`Value::Number`
/// never equals `Value::String`) but could compare true in SQL against an
/// unquoted text cell, and a boolean has the same problem. Those stay in
/// Rust and make the plan incomplete, so the count stays honest.
#[test]
fn a_non_string_hash_map_eq_over_an_indexed_field_stays_residual() {
    let indexed = ["device_id".to_string()];
    for value in [Value::from(1), Value::Bool(true), Value::Null] {
        let mut filters = HashMap::new();
        filters.insert("device_id".to_string(), value.clone());
        let (plan, residual) = Plan::build(&filters, &[], None, None, &indexed);
        assert!(plan.filters_nothing(), "{value:?}");
        assert_eq!(residual.len(), 1, "{value:?}: {residual:?}");
        assert_eq!(residual[0].reason, ResidualReason::JsonExactNotPushed);
        assert!(!plan.is_complete(), "{value:?}");
    }
}
