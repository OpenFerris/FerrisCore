use super::*;

#[test]
fn a_column_field_is_pushed_down_and_a_blob_field_is_reported() {
    let (plan, residual) = Plan::build(&filters(&[("owner", "ada")]), &[], None, None, &[]);
    assert!(!plan.filters_nothing());
    assert!(residual.is_empty(), "{residual:?}");

    let (_, residual) = Plan::build(&filters(&[("title", "x")]), &[], None, None, &[]);
    assert_eq!(residual.len(), 1);
    assert_eq!(residual[0].field, "title");
    assert_eq!(residual[0].reason, ResidualReason::NotAColumn);
}

/// The property that decides whether SQL may page: a plan is only complete
/// when nothing was left behind.
#[test]
fn a_plan_with_any_residual_filter_is_not_complete() {
    let (only_columns, residual) = Plan::build(&filters(&[("owner", "ada")]), &[], None, None, &[]);
    assert!(residual.is_empty());
    assert!(only_columns.is_complete(), "nothing was left in Rust");

    let (with_blob, residual) = Plan::build(&filters(&[("title", "x")]), &[], None, None, &[]);
    assert!(!residual.is_empty());
    assert!(
        !with_blob.is_complete(),
        "COUNT(*) over a partial filter would report a total the caller cannot reconcile"
    );
}

#[test]
fn search_is_always_residual_because_it_walks_the_blob() {
    let (plan, residual) = Plan::build(&HashMap::new(), &[], Some("needle"), None, &[]);
    assert!(plan.filters_nothing());
    assert_eq!(residual.len(), 1);
    assert_eq!(residual[0].field, "<search>");
    assert!(!plan.is_complete());
}

/// A blank search is not a filter, so it must not be reported as one.
#[test]
fn a_blank_search_is_not_a_residual() {
    let (_, residual) = Plan::build(&HashMap::new(), &[], Some("   "), None, &[]);
    assert!(residual.is_empty(), "{residual:?}");
}

#[test]
fn ordering_by_a_column_becomes_order_by_sql_and_by_a_blob_field_does_not() {
    let (plan, residual) = Plan::build(&HashMap::new(), &[], None, Some("modified desc"), &[]);
    assert!(!plan.orders_in_rust());
    assert!(residual.is_empty(), "{residual:?}");

    let (plan, residual) = Plan::build(&HashMap::new(), &[], None, Some("title"), &[]);
    assert!(plan.orders_in_rust());
    assert_eq!(residual[0].reason, ResidualReason::OrderByNotAColumn);
}

/// With no `order_by` the plan still has to order — `ORDER BY modified DESC`
/// is what the engine always used, and dropping it would let the database
/// return rows in whatever order the plan happened to pick.
#[test]
fn an_unordered_request_still_orders_by_modified_descending() {
    for (plan, _) in [
        Plan::build(&HashMap::new(), &[], None, None, &[]),
        Plan::build(&filters(&[("title", "x")]), &[], None, None, &[]),
        Plan::build(&HashMap::new(), &[], None, Some("   "), &[]),
    ] {
        let sql = plan
            .select_all("tabToDo", Dialect::SQLite)
            .unwrap()
            .sql()
            .to_string();
        assert!(sql.contains("\"modified\" DESC"), "{sql}");
    }
}

/// An explicit order replaces the default rather than adding to it, and the
/// order stays total either way: `LIMIT`/`OFFSET` needs a total order or
/// two identical requests can return the same row twice. Ordering by
/// `name` is already total, so the tiebreaker is not repeated.
#[test]
fn paging_is_total_ordered_whatever_the_request_asked_for() {
    let (plan, _) = Plan::build(&HashMap::new(), &[], None, Some("name asc"), &[]);
    let sql = plan
        .select_page("tabToDo", Dialect::Postgres, 5, 5)
        .unwrap()
        .sql()
        .to_string();
    assert!(sql.contains("ORDER BY \"name\" ASC"), "{sql}");
    assert!(!sql.contains("ASC, \"name\""), "{sql}");

    let (plan, _) = Plan::build(&HashMap::new(), &[], None, None, &[]);
    let sql = plan
        .select_page("tabToDo", Dialect::Postgres, 5, 5)
        .unwrap()
        .sql()
        .to_string();
    // The tiebreaker is the `name` column, never the table identifier: the
    // latter is not a column and fails hard on PostgreSQL.
    assert!(
        sql.contains("ORDER BY \"modified\" DESC, \"name\" ASC"),
        "{sql}"
    );
}

/// Every column name this module can emit has to survive identifier
/// validation: a predicate on a name outside `COLUMN_FIELDS` never reaches
/// `render`, but `with_equality` is a public entry point and does.
#[test]
fn an_extra_predicate_cannot_name_an_arbitrary_column() {
    assert!(
        Plan::default()
            .with_equality("owner", Value::string("ada"))
            .is_ok()
    );
    assert!(
        Plan::default()
            .with_equality("data; DROP TABLE tabToDo", Value::Null)
            .is_err()
    );
    assert!(Plan::default().with_equality("", Value::Null).is_err());
}

#[test]
fn page_selects_with_bound_limit_and_offset() {
    let (plan, _) = Plan::build(&HashMap::new(), &[], None, None, &[]);
    for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
        let stmt = plan.select_page("tabToDo", dialect, 20, 40).unwrap();
        let sql = stmt.sql();
        assert!(sql.contains("LIMIT ? OFFSET ?"), "{dialect:?}: {sql}");
        // Paging is always total-ordered, so two identical requests return
        // the same rows instead of overlapping pages.
        assert!(sql.contains("ORDER BY"), "{dialect:?}: {sql}");
        assert!(sql.ends_with("LIMIT ? OFFSET ?"), "{dialect:?}: {sql}");
    }
}

/// The unpaged form must not carry a `LIMIT`: the engine drops rows after
/// this, and a limit here would shorten a page twice.
#[test]
fn the_unpaged_select_carries_no_limit() {
    let (plan, _) = Plan::build(&HashMap::new(), &[], None, None, &[]);
    let sql = plan
        .select_all("tabToDo", Dialect::SQLite)
        .unwrap()
        .sql()
        .to_string();
    assert!(!sql.contains("LIMIT"), "{sql}");
}

#[test]
fn count_counts_the_pushed_down_subset() {
    let (plan, _) = Plan::build(&filters(&[("owner", "ada")]), &[], None, None, &[]);
    let sql = plan
        .count("tabToDo", Dialect::Postgres)
        .unwrap()
        .sql()
        .to_string();
    assert!(
        sql.starts_with("SELECT COUNT(*) AS n FROM \"tabToDo\""),
        "{sql}"
    );
    assert!(sql.contains("WHERE"), "{sql}");
}

/// The needle travels as a bound parameter with the wildcards added by the
/// builder, and the match is case-insensitive on every dialect — the Rust
/// filter it replaces lowercases both sides.
#[test]
fn a_like_needle_binds_wildcards_and_escapes_the_users_own() {
    let (plan, _) = Plan::build(
        &HashMap::new(),
        &[op("owner", FilterOp::Like, "50%_off")],
        None,
        None,
        &[],
    );
    for (dialect, expected) in [
        (Dialect::SQLite, "LOWER(\"owner\") LIKE LOWER(?)"),
        (Dialect::MySQL, "LOWER(`owner`) LIKE LOWER(?)"),
    ] {
        let sql = plan
            .select_all("tabToDo", dialect)
            .unwrap()
            .sql()
            .to_string();
        assert!(sql.contains(expected), "{dialect:?}: {sql}");
    }
    let sql = plan
        .select_all("tabToDo", Dialect::Postgres)
        .unwrap()
        .sql()
        .to_string();
    assert!(sql.contains("\"owner\" ILIKE ?"), "{sql}");
}

/// `escape_like` exists so a literal `%` searches for a percent sign.
#[test]
fn a_literal_wildcard_in_a_needle_is_escaped() {
    assert_eq!(escape_like("100%"), "100\\%");
    assert_eq!(escape_like("a_b"), "a\\_b");
    assert_eq!(escape_like("back\\slash"), "back\\\\slash");
}

/// An empty `in` list must match nothing; `IN ()` is a syntax error on all
/// three dialects, so it is spelled as a contradiction.
#[test]
fn an_empty_in_list_matches_nothing() {
    let (plan, _) = Plan::build(
        &HashMap::new(),
        &[op("owner", FilterOp::In, " , , ")],
        None,
        None,
        &[],
    );
    let sql = plan
        .select_all("tabToDo", Dialect::SQLite)
        .unwrap()
        .sql()
        .to_string();
    assert!(sql.contains("1 = 0"), "{sql}");
}

#[test]
fn an_in_list_expands_to_one_placeholder_per_value() {
    let (plan, _) = Plan::build(
        &HashMap::new(),
        &[op("owner", FilterOp::In, "ada,grace, bob")],
        None,
        None,
        &[],
    );
    let sql = plan
        .select_all("tabToDo", Dialect::Postgres)
        .unwrap()
        .sql()
        .to_string();
    assert!(sql.contains("IN (?, ?, ?)"), "{sql}");
}

/// `matches_filters` ANDs within a group and ORs across groups. Appending one
/// term of a disjunction to a `WHERE ... AND` turns "A OR B" into "A AND B",
/// so every row that matched only through the other side disappears. The
/// shape that triggers it is the Desk's own `[[f, op, v, or], …]` syntax.
#[test]
fn an_or_group_is_not_pushed_down_as_a_conjunction() {
    // `qty <= 1` (blob field) OR `name like P-2` (a real column).
    let ops = vec![
        ListFilter {
            field: "qty".to_string(),
            op: FilterOp::Lte,
            value: Value::string("1"),
            or: false,
        },
        ListFilter {
            field: "name".to_string(),
            op: FilterOp::Like,
            value: Value::string("P-2"),
            or: true,
        },
    ];
    let (plan, residual) = Plan::build(&HashMap::new(), &ops, None, None, &[]);
    assert!(
        plan.filters_nothing(),
        "a disjunction cannot become a WHERE clause"
    );
    assert_eq!(residual.len(), 2, "{residual:?}");
    for r in &residual {
        assert_eq!(r.reason, ResidualReason::Disjunction, "{r:?}");
    }
    let sql = plan
        .select_all("tabToDo", Dialect::SQLite)
        .unwrap()
        .sql()
        .to_string();
    assert!(!sql.contains("WHERE"), "{sql}");
}

/// The mirror case: a leading `or` with nothing to end is a single group, so
/// the filters *are* a conjunction and may be pushed.
#[test]
fn a_leading_or_flag_is_still_a_single_conjunction() {
    let ops = vec![
        ListFilter {
            field: "name".to_string(),
            op: FilterOp::Like,
            value: Value::string("P"),
            or: true,
        },
        ListFilter {
            field: "owner".to_string(),
            op: FilterOp::Equals,
            value: Value::string("ada@local"),
            or: false,
        },
    ];
    assert_eq!(group_count(&ops), 1);
    let (plan, residual) = Plan::build(&HashMap::new(), &ops, None, None, &[]);
    assert!(residual.is_empty(), "{residual:?}");
    let sql = plan
        .select_all("tabToDo", Dialect::SQLite)
        .unwrap()
        .sql()
        .to_string();
    assert!(sql.contains("WHERE"), "{sql}");
}

/// `group_count` mirrors [`crate::filters::matches_filters`]' grouping, and a
/// drift between the two would push a filter the evaluator then re-applies
/// under a different grouping — a silently wrong row set. This checks the
/// mirror against the real evaluator over every combination of matches,
/// rather than against a second copy of the rule that could drift the same
/// way.
///
/// Each filter gets its own field, so a document is fully described by which
/// filters it hits and the two models can be compared exhaustively.
#[test]
fn the_group_count_matches_the_filter_evaluator() {
    // Every `or`-pattern up to three filters.
    for flags in [
        vec![false],
        vec![true],
        vec![false, false],
        vec![false, true],
        vec![true, false],
        vec![true, true],
        vec![false, false, false],
        vec![false, false, true],
        vec![false, true, false],
        vec![false, true, true],
        vec![true, false, false],
        vec![true, false, true],
        vec![true, true, false],
        vec![true, true, true],
    ] {
        let ops: Vec<ListFilter> = flags
            .iter()
            .enumerate()
            .map(|(i, or)| ListFilter {
                field: format!("f{i}"),
                op: FilterOp::Equals,
                value: Value::string("hit"),
                or: *or,
            })
            .collect();

        let boundaries = crate::filters::or_groups(&ops);
        // The model the plan relies on: `matches_filters` as an OR over groups,
        // each group an AND over its members.
        let modelled =
            |hits: &[bool]| -> bool { boundaries.iter().any(|g| g.iter().all(|i| hits[*i])) };

        for mask in 0..1u32 << ops.len() {
            let hits: Vec<bool> = (0..ops.len()).map(|i| mask >> i & 1 == 1).collect();
            let doc = Value::Object(
                hits.iter()
                    .enumerate()
                    .map(|(i, hit)| {
                        (
                            format!("f{i}"),
                            Value::string(if *hit { "hit" } else { "miss" }),
                        )
                    })
                    .collect(),
            );
            assert_eq!(
                crate::filters::matches_filters(&doc, &HashMap::new(), &ops),
                modelled(&hits),
                "flags {flags:?} hits {hits:?}"
            );
        }
        // The count the plan branches on is the number of groups the boundaries
        // describe, so the two can never disagree.
        assert_eq!(group_count(&ops), boundaries.len(), "flags {flags:?}");
    }
}

/// A column that does not exist must never reach SQL, because it would be a
/// runtime error instead of a slower-but-correct list.
#[test]
fn a_filter_on_a_blob_field_never_becomes_a_column_predicate() {
    let (plan, residual) = Plan::build(
        &HashMap::new(),
        &[op("title", FilterOp::Equals, "x")],
        None,
        None,
        &[],
    );
    assert!(plan.filters_nothing());
    assert_eq!(residual.len(), 1);
    let sql = plan
        .select_all("tabToDo", Dialect::SQLite)
        .unwrap()
        .sql()
        .to_string();
    assert!(!sql.contains("WHERE"), "{sql}");
}

/// `docstatus` is `SMALLINT` on disk and the *word* `draft` in `data`;
/// `creation`/`modified` are epoch `BIGINT`s on disk and RFC 3339 strings in
/// `data`. The plan now maps the value it read to the column representation
/// (word→SMALLINT, RFC3339→epoch) and pushes the predicate, instead of
/// silently filtering the whole table in Rust.
#[test]
fn a_column_that_stores_a_different_representation_is_mapped_to_its_column_value() {
    for (field, as_read) in [
        ("docstatus", "draft"),
        ("creation", "2026-01-01T09:00:00Z"),
        ("modified", "2026-01-01T09:00:00Z"),
    ] {
        assert!(is_column_field(field), "{field} does have a column");
        let (plan, residual) = Plan::build(
            &HashMap::new(),
            &[op(field, FilterOp::Equals, as_read)],
            None,
            None,
            &[],
        );
        assert!(
            !plan.filters_nothing(),
            "{field} = {as_read} must be pushed with the mapped value"
        );
        assert!(residual.is_empty(), "{field}: {residual:?}");
        // `ORDER BY "modified"` is the plan's default sort, so the check is
        // on the `WHERE` body specifically.
        let sql = plan
            .select_all("tabToDo", Dialect::SQLite)
            .unwrap()
            .sql()
            .to_string();
        let where_sql = sql.split(" ORDER BY ").next().unwrap_or_default();
        assert!(
            where_sql.contains(field),
            "{field} must appear in WHERE: {sql}"
        );
    }
}

/// Those three are still fine as `ORDER BY` keys: both representations
/// increase with time, so the ordering agrees even though the values do not.
#[test]
fn a_differently_represented_column_can_still_order_in_sql() {
    for field in ["docstatus", "creation", "modified"] {
        let (plan, residual) = Plan::build(&HashMap::new(), &[], None, Some(field), &[]);
        assert!(!plan.orders_in_rust(), "{field} should order in SQL");
        assert!(residual.is_empty(), "{field}: {residual:?}");
    }
}

#[test]
fn only_real_columns_are_accepted() {
    for field in ["name", "owner", "docstatus", "creation", "modified"] {
        assert!(is_column_field(field), "{field} is a real column");
    }
    for field in ["title", "status", "data", "name ", "Name"] {
        assert!(!is_column_field(field), "{field} is not");
    }
}

/// And of the five columns, only two are filterable — see
/// [`FILTERABLE_FIELDS`] for why the other three are order-only.
#[test]
fn only_text_verbatim_columns_are_filterable() {
    for field in ["name", "owner"] {
        assert!(
            is_filterable_field(field),
            "{field} holds the document's value"
        );
    }
    for field in ["docstatus", "creation", "modified"] {
        assert!(
            !is_filterable_field(field),
            "{field} holds another representation"
        );
    }
}
