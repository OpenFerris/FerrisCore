//! Structured list filters evaluated in Rust over parsed documents.
//!
//! SQL answers the filters it can (see [`crate::list_sql`]); the ones that
//! cannot be pushed down are evaluated in memory over the reconstructed
//! document. These helpers evaluate the operator-based filters the Desk sends
//! (`[[field, op, value, or?], …]`) and sort documents by an `order_by`
//! expression. Legacy equality maps (`GET /api/resource` query params) keep
//! their own HashMap path in [`crate::engine::ListQuery`].

use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{NaiveDate, NaiveDateTime, Timelike};
use ferris_core::Value;

/// Comparison operator of a structural list filter.
///
/// The tokens are the contract: the Desk writes them into the `filters` string
/// and [`parse_list_filters`] reads the same ones back, so the enum itself
/// lives in `ferris-schema::contract` (where the client also has it) rather
/// than being a second, independently-editable copy.
pub use ferris_schema::contract::list::FilterOp;

/// One operator-based filter bound to a document field.
#[derive(Debug, Clone, PartialEq)]
pub struct ListFilter {
    pub field: String,
    pub op: FilterOp,
    pub value: Value,
    /// When `true`, the filter starts a new OR group (rows before it stay in
    /// their own group; groups are joined with OR).
    pub or: bool,
}

/// Parse the Desk list `filters` JSON: `[[field, op, value, or?], …]`.
///
/// Unparseable rows are skipped so a malformed filter never breaks a list
/// request; last element of each row (the OR flag) is optional.
pub fn parse_list_filters(json: &str) -> Vec<ListFilter> {
    let mut ops: Vec<ListFilter> = Vec::new();

    let Ok(serde_json::Value::Array(rows)) = serde_json::from_str(json) else {
        return ops;
    };
    for row in rows {
        let Some(items) = row.as_array() else {
            continue;
        };
        let Some(field) = items.first().and_then(|v| v.as_str()) else {
            continue;
        };
        if field.is_empty() {
            continue;
        }
        let op = items
            .get(1)
            .and_then(|v| v.as_str())
            .and_then(FilterOp::from_str)
            .unwrap_or(FilterOp::Equals);
        let raw = items.get(2).cloned().unwrap_or(serde_json::Value::Null);
        ops.push(ListFilter {
            field: field.to_string(),
            op,
            value: Value::string(cell_str(Some(&Value::from_json(raw)))),
            or: items.get(3).and_then(|v| v.as_bool()).unwrap_or(false),
        });
    }
    ops
}

/// Evaluate the legacy equality map (AND) together with the operator filters
/// (rows grouped with OR).
pub fn matches_filters(
    value: &Value,
    equality: &HashMap<String, Value>,
    ops: &[ListFilter],
) -> bool {
    let eq_ok = equality.iter().all(|(key, expected)| {
        value
            .get(key)
            .map(|actual| actual == expected)
            .unwrap_or(false)
    });
    if !eq_ok {
        return false;
    }
    if ops.is_empty() {
        return true;
    }

    let groups = or_groups(ops);
    groups
        .iter()
        .any(|g| g.iter().all(|&i| single_matches(value, &ops[i])))
}

/// The indices of each OR group in `ops`.
///
/// This is the single definition of the OR-grouping rule: `matches_filters`
/// evaluates a row as "any group matches fully" and `list_sql` uses it to decide
/// what the whole group must push into SQL. A filter carrying `or` ends the
/// current group, except at the start, where there is nothing to end (DOC5 — the
/// evaluator and the planner used to re-derive this rule separately, so a drift
/// silently changed which rows a list returned).
pub fn or_groups(ops: &[ListFilter]) -> Vec<Vec<usize>> {
    ferris_schema::contract::list::or_groups(ops.iter().map(|filter| filter.or))
}

fn single_matches(value: &Value, f: &ListFilter) -> bool {
    let cell = cell_str(value.get(&f.field));
    let needle = cell_str(Some(&f.value));

    // A filter is only as precise as the control that produced it: a `Date`
    // filter names a day and a `datetime-local` filter names a minute, while
    // the stored value carries seconds and possibly an offset. Comparing the
    // raw strings made `equals 12:34` miss every `12:34:56` row and put
    // `lte 12:34` *before* the minute the user picked. Comparing the parsed
    // instants at the needle's precision is the one place that can fix all
    // five operators without the Desk having to guess the stored format.
    if let Some(ord) = timestamp_cmp(&cell, &needle) {
        match f.op {
            FilterOp::Equals => return ord == Ordering::Equal,
            FilterOp::Gt => return ord == Ordering::Greater,
            FilterOp::Gte => return ord != Ordering::Less,
            FilterOp::Lt => return ord == Ordering::Less,
            FilterOp::Lte => return ord != Ordering::Greater,
            // Substring and membership keep their text semantics below.
            FilterOp::Like | FilterOp::In => {}
        }
    }

    match f.op {
        FilterOp::Equals => cell == needle,
        FilterOp::Like => cell.to_lowercase().contains(&needle.to_lowercase()),
        FilterOp::In => needle.split(',').any(|part| cell == part.trim()),
        FilterOp::Gt => cmp(&cell, &needle) == Ordering::Greater,
        FilterOp::Gte => !matches!(cmp(&cell, &needle), Ordering::Less),
        FilterOp::Lt => cmp(&cell, &needle) == Ordering::Less,
        FilterOp::Lte => !matches!(cmp(&cell, &needle), Ordering::Greater),
    }
}

/// How much of a timestamp a filter value names.
///
/// The precision comes from the format the needle was written in, never from
/// the cell: a stored `12:34:56` filtered with `equals 12:34` must match the
/// whole minute, and the same stored value filtered with `equals 12:34:56`
/// must still be an exact second.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Precision {
    Day,
    Minute,
    Second,
}

/// The naive timestamp spellings Ferris writes and accepts.
///
/// `%Y-%m-%dT%H:%M` is what `<input type="datetime-local">` submits; the
/// second-precision and space-separated forms are what the engine and the
/// validator already store.
const TIMESTAMP_FORMATS: &[(&str, Precision)] = &[
    ("%Y-%m-%dT%H:%M:%S", Precision::Second),
    ("%Y-%m-%dT%H:%M:%S%.f", Precision::Second),
    ("%Y-%m-%dT%H:%M", Precision::Minute),
    ("%Y-%m-%d %H:%M:%S", Precision::Second),
    ("%Y-%m-%d %H:%M:%S%.f", Precision::Second),
    ("%Y-%m-%d %H:%M", Precision::Minute),
];

/// Parse the timestamp spellings Ferris stores and reads.
///
/// `parse_from_rfc3339` keeps the written wall clock (`naive_local`) instead of
/// converting to UTC: the Desk stores a `Datetime` the same way and has no
/// per-user conversion yet, so shifting the cell here would make a value and
/// the filter the user pointed at it disagree whenever the offset is not zero.
fn parse_timestamp(text: &str) -> Option<(NaiveDateTime, Precision)> {
    let text = text.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some((dt.naive_local(), Precision::Second));
    }
    for (format, precision) in TIMESTAMP_FORMATS {
        if let Ok(dt) = NaiveDateTime::parse_from_str(text, format) {
            return Some((dt, *precision));
        }
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|dt| (dt, Precision::Day))
}

/// Truncate `dt` to `precision`, dropping the fields the filter did not name.
fn truncate(dt: NaiveDateTime, precision: Precision) -> Option<NaiveDateTime> {
    match precision {
        Precision::Day => dt.date().and_hms_opt(0, 0, 0),
        Precision::Minute => dt.with_second(0)?.with_nanosecond(0),
        Precision::Second => dt.with_nanosecond(0),
    }
}

/// Compare two timestamp strings at the precision the needle names.
///
/// `None` means at least one side is not a timestamp, so the caller keeps the
/// text/number comparison it had before.
fn timestamp_cmp(cell: &str, needle: &str) -> Option<Ordering> {
    let (cell, _) = parse_timestamp(cell)?;
    let (needle, precision) = parse_timestamp(needle)?;
    Some(truncate(cell, precision)?.cmp(&truncate(needle, precision)?))
}

/// Whether `text` is a timestamp [`timestamp_cmp`] will compare by precision.
///
/// The SQL planner uses this to keep an exact predicate over an indexed JSON
/// field in Rust: the expression index compares raw text, so pushing a minute
/// would drop the `:SS` rows the Rust evaluator keeps.
pub(crate) fn is_timestamp_value(text: &str) -> bool {
    parse_timestamp(text).is_some()
}

/// Stable sort of parsed documents by an `order_by` expression
/// (`"field"`, `"field asc"` or `"field desc"`). Missing/null cells sort last
/// regardless of direction.
pub fn order_documents(mut documents: Vec<Value>, order_by: &str) -> Vec<Value> {
    let mut tokens = order_by.split_whitespace();
    let Some(field) = tokens.next() else {
        return documents;
    };
    if field.is_empty() {
        return documents;
    }
    let desc = tokens
        .next()
        .map(|t| t.eq_ignore_ascii_case("desc"))
        .unwrap_or(false);

    let empty_last = true;
    documents.sort_by(|a, b| {
        let (sa, sb) = (cell_str(a.get(field)), cell_str(b.get(field)));
        let (ea, eb) = (sa.is_empty(), sb.is_empty());
        match (ea, eb) {
            (true, false) => {
                return if empty_last {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
            (false, true) => {
                return if empty_last {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            _ => {}
        }
        let ord = match (sa.parse::<f64>(), sb.parse::<f64>()) {
            (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            _ => sa.to_lowercase().cmp(&sb.to_lowercase()),
        };
        if desc { ord.reverse() } else { ord }
    });
    documents
}

/// Coerce a cell to its display string (numbers, booleans, objects).
fn cell_str(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// Numeric-aware comparison that falls back to lexicographic for text.
fn cmp(a: &str, b: &str) -> Ordering {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Value {
        Value::from_json(
            serde_json::from_str::<serde_json::Value>(
                r#"{"name":"T-1","title":"Hello World","status":"open","qty":3.0,"flag":true}"#,
            )
            .unwrap(),
        )
    }

    #[test]
    fn parses_structured_rows() {
        let ops = parse_list_filters(r#"[["status","equals","open"],["qty","gte","2"]]"#);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].op, FilterOp::Equals);
        assert_eq!(ops[1].op, FilterOp::Gte);
    }

    #[test]
    fn skips_malformed_rows() {
        let ops = parse_list_filters(r#"[0,["qty","gte","2"],["qty","lte","4"]]"#);
        assert_eq!(ops.len(), 2);
    }

    #[test]
    fn or_groups_join_with_or() {
        let ops = parse_list_filters(
            r#"[["title","equals","Goodbye",false],["title","like","hello",true]]"#,
        );
        let eq = HashMap::new();
        assert!(matches_filters(&doc(), &eq, &ops));
    }

    /// DOC5: `or_groups` is the one definition of the OR-grouping rule; the
    /// evaluator (`matches_filters`) and the SQL planner both read it.
    #[test]
    fn or_groups_splits_on_the_or_flag_only_after_the_first_row() {
        // A leading `or` does not open an empty group (nothing to end yet).
        let ops = parse_list_filters(
            r#"[["qty","gt","0",true],["qty","lt","10"],["status","equals","open",true]]"#,
        );
        assert_eq!(
            or_groups(&ops),
            vec![vec![0, 1], vec![2]],
            "a leading `or` must not create an empty group"
        );

        // No `or` at all: one AND group.
        let ops = parse_list_filters(r#"[["qty","gt","0"],["qty","lt","10"]]"#);
        assert_eq!(or_groups(&ops), vec![vec![0, 1]]);
    }

    #[test]
    fn and_rows_must_all_match() {
        let ops = parse_list_filters(r#"[["qty","gte","2"],["qty","lt","4"]]"#);
        assert!(matches_filters(&doc(), &HashMap::new(), &ops));

        let ops = parse_list_filters(r#"[["qty","gt","50"]]"#);
        assert!(!matches_filters(&doc(), &HashMap::new(), &ops));

        let ops = parse_list_filters(r#"[["status","in","open,closed"]]"#);
        assert!(matches_filters(&doc(), &HashMap::new(), &ops));
    }

    #[test]
    fn legacy_equality_map_applies_as_and() {
        let mut eq = HashMap::new();
        eq.insert("status".to_string(), Value::string("open"));
        let ops = parse_list_filters(r#"[["qty","gte","2"]]"#);
        assert!(matches_filters(&doc(), &eq, &ops));

        eq.insert("status".to_string(), Value::string("closed"));
        assert!(!matches_filters(&doc(), &eq, &ops));
    }

    #[test]
    fn orders_by_numeric_and_text() {
        let a: Value = Value::from_json(serde_json::json!({"name":"A","qty":10,"rank":"beta"}));
        let b: Value = Value::from_json(serde_json::json!({"name":"B","qty":2,"rank":"alpha"}));
        let c: Value = Value::from_json(serde_json::json!({"name":"C","qty":7,"rank":"Gamma"}));

        let out = order_documents(vec![a.clone(), b.clone(), c.clone()], "qty desc");
        assert_eq!(out[0].get("name").and_then(|v| v.as_str()), Some("A"));
        assert_eq!(out[2].get("name").and_then(|v| v.as_str()), Some("B"));

        let out = order_documents(vec![a, b, c], "rank");
        assert_eq!(out[0].get("name").and_then(|v| v.as_str()), Some("B"));
    }

    /// The Desk picker names a minute, while the stored stamp carries seconds
    /// and an offset. `equals` used to compare the two literals, so a datetime
    /// filter never matched anything.
    #[test]
    fn datetime_equals_matches_the_minute_the_filter_names() {
        let cell = |stamp: &str| Value::from_json(serde_json::json!({ "modified": stamp }));
        let ops = parse_list_filters(r#"[["modified","equals","2026-10-30T12:34"]]"#);

        for stamp in ["2026-10-30T12:34:56+00:00", "2026-10-30T12:34:00Z"] {
            assert!(
                matches_filters(&cell(stamp), &HashMap::new(), &ops),
                "{stamp} is inside the selected minute"
            );
        }
        assert!(!matches_filters(
            &cell("2026-10-30T12:35:00+00:00"),
            &HashMap::new(),
            &ops
        ));
    }

    /// The named minute is a whole bucket: `gt`/`gte` start after it and
    /// `lt`/`lte` end with it, so the five operators stay distinct instead of
    /// collapsing into the lexicographic prefix comparison.
    #[test]
    fn datetime_ranges_compare_the_whole_named_minute() {
        let cell = Value::from_json(serde_json::json!({
            "modified": "2026-10-30T12:34:56+00:00"
        }));
        for (op, expected) in [("gt", false), ("gte", true), ("lt", false), ("lte", true)] {
            let raw = format!(r#"[["modified","{op}","2026-10-30T12:34"]]"#);
            let ops = parse_list_filters(&raw);
            assert_eq!(
                matches_filters(&cell, &HashMap::new(), &ops),
                expected,
                "{op} over the 12:34 minute"
            );
        }
    }

    /// A `Date` filter names a day, so it has to match every stamp inside it.
    #[test]
    fn date_filter_matches_the_whole_named_day() {
        let cell = Value::from_json(serde_json::json!({
            "modified": "2026-10-30T23:59:59+00:00"
        }));
        let ops = parse_list_filters(r#"[["modified","equals","2026-10-30"]]"#);
        assert!(matches_filters(&cell, &HashMap::new(), &ops));
    }

    /// A value that is not a timestamp keeps the text/number comparison it had,
    /// so this fix cannot change an ordinary string filter.
    #[test]
    fn non_timestamp_values_keep_their_old_comparison() {
        let ops = parse_list_filters(r#"[["title","equals","Hello World"]]"#);
        assert!(matches_filters(&doc(), &HashMap::new(), &ops));
        let ops = parse_list_filters(r#"[["title","equals","hello"]]"#);
        assert!(!matches_filters(&doc(), &HashMap::new(), &ops));
    }
}
