//! Client-side structural filtering over already-fetched rows.

use serde_json::Value;
use std::borrow::Cow;

/// Whether a row satisfies a structural filter (string coercion; numeric for ranges).
pub fn matches_filter(row: &Value, f: &crate::types::Filter) -> bool {
    // The common case is a string cell, which is borrowed: comparing in place
    // keeps a filter pass allocation-free instead of building one String per
    // cell per filter.
    let borrowed = match row.get(&f.field) {
        Some(Value::String(s)) => Cow::Borrowed(s.as_str()),
        other => Cow::Owned(cell_as_str(other)),
    };
    let cell = borrowed.as_ref();
    let needle = f.value.trim();
    match f.op {
        crate::types::FilterOp::Equals => cell == needle,
        crate::types::FilterOp::Like => contains_ignore_case(cell, needle),
        crate::types::FilterOp::In => needle.split(',').any(|p| cell == p.trim()),
        crate::types::FilterOp::Gt => num_cmp(cell, needle).is_gt(),
        crate::types::FilterOp::Gte => num_cmp(cell, needle).is_ge(),
        crate::types::FilterOp::Lt => num_cmp(cell, needle).is_lt(),
        crate::types::FilterOp::Lte => num_cmp(cell, needle).is_le(),
    }
}

/// `haystack` contains `needle`, case-insensitively.
///
/// The ASCII path folds case byte by byte over a sliding window, so a filter
/// pass allocates nothing per cell; only a non-ASCII pair falls back to the
/// `to_lowercase` comparison (which is also the only correct path for the
/// multi-byte case foldings `eq_ignore_ascii_case` cannot express).
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle.is_ascii() {
        let needle = needle.as_bytes();
        return haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle));
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn cell_as_str(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn num_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.cmp(b),
    }
}

/// Client-side structural filtering so hydrate requests stay simple (equality at engine).
///
/// Filters are evaluated as OR groups, exactly like the engine: a filter with
/// `or: true` starts a new group, and a row passes when **any** group matches
/// fully. The grouping rule itself is
/// [`ferris_schema::contract::list::or_groups`], shared with
/// `ferris_document::filters`, so the local path cannot drift from the SQL one
/// (WS9a).
pub fn apply_filters(rows: Vec<Value>, filters: &[crate::types::Filter]) -> Vec<Value> {
    if filters.is_empty() {
        return rows;
    }
    let groups = ferris_schema::contract::list::or_groups(filters.iter().map(|f| f.or));
    rows.into_iter()
        .filter(|row| {
            groups.iter().any(|group| {
                group
                    .iter()
                    .all(|&index| matches_filter(row, &filters[index]))
            })
        })
        .collect()
}
