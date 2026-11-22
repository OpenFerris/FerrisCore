//! List request and response: the body of `POST /api/resource/{doctype}/list`
//! and the envelope both platforms deserialize.
//!
//! One type serves all three consumers that used to disagree about the shape:
//! the Desk builds it, the REST handler deserializes it, and the SSR in-process
//! bridge forwards it to the engine without re-mapping fields.

use serde::{Deserialize, Serialize};

/// Desk-style list request body for `POST /api/resource/{doctype}/list`.
///
/// `filters` is the structural JSON `[[field, op, value, or?], …]` string and
/// `fields` is a projection list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListRequest {
    #[serde(default)]
    pub limit_start: u64,
    #[serde(default = "default_page_length")]
    pub limit_page_length: u64,
    #[serde(default)]
    pub filters: Option<String>,
    #[serde(default)]
    pub order_by: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    /// Optional field projection; `None` keeps the doctype defaults.
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Keep only the documents the requesting user has liked.
    ///
    /// A like lives in `tab_like`, not in a document field, so it cannot be
    /// expressed through `filters`; the store resolves it to a set of names
    /// before the engine runs, which keeps `total` and the pagination honest.
    ///
    /// `skip_serializing_if` matters: this type is both the body the Desk posts
    /// and the struct the handler deserializes, and almost no caller sets the
    /// flag. Omitting it is the same as sending `null`, but sending `"only_liked":
    /// null` in every list body is noise on a hot path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_liked: Option<bool>,
}

/// Rows plus the pagination the UI needs to render a pager.
///
/// Both the SSR bridge and the REST client deserialize their list response
/// into this same envelope, so a view never has to know which platform it is
/// running on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

/// The documents adjacent to one document in the doctype's list order
/// (`GET /api/resource/{doctype}/{name}/neighbors`).
///
/// Both fields are `None` when there is no such document, which is what lets
/// the form's previous/next controls disable themselves rather than offering a
/// link that goes nowhere.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Neighbors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// A structural filter built from doctype metadata.
///
/// This is the *client* shape: the Desk composes these from the metadata
/// dropdowns and hands them to [`filters_json`], which produces the string the
/// engine parses. `ferris_document::ListFilter` is the server-side parse result
/// and is deliberately a separate type — it carries a `ferris_core::Value`
/// cell, which this module cannot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Filter {
    pub field: String,
    pub op: FilterOp,
    pub value: String,
    /// When `true` the filter starts a new OR group on the server.
    #[serde(default)]
    pub or: bool,
}

impl Filter {
    pub fn tuple(&self) -> (String, String, String, bool) {
        (
            self.field.clone(),
            self.op.as_str().to_string(),
            self.value.clone(),
            self.or,
        )
    }
}

/// Comparison operators the Desk offers, in the order the filter builder lists
/// them.
pub const FILTER_OPERATORS: &[(&str, FilterOp)] = &[
    ("Equals", FilterOp::Equals),
    ("Is Like", FilterOp::Like),
    ("Is In", FilterOp::In),
    ("Greater Than", FilterOp::Gt),
    ("Greater Than Or Equal", FilterOp::Gte),
    ("Less Than", FilterOp::Lt),
    ("Less Than Or Equal", FilterOp::Lte),
];

/// Comparison operator of a structural list filter.
///
/// The string form is the wire token in a filter row and the input to
/// [`FilterOp::from_str`]; the engine parses the same tokens into its own
/// `FilterOp` (`ferris_document::filters`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FilterOp {
    Like,
    Equals,
    In,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl FilterOp {
    pub fn as_str(self) -> &'static str {
        match self {
            FilterOp::Like => "like",
            FilterOp::Equals => "equals",
            FilterOp::In => "in",
            FilterOp::Gt => "gt",
            FilterOp::Gte => "gte",
            FilterOp::Lt => "lt",
            FilterOp::Lte => "lte",
        }
    }

    /// Parse a wire token, case-insensitively.
    ///
    /// Lenient on purpose: these rows can come back from persisted view state,
    /// which is user-editable JSON, and `Is Like` is the label a human writes.
    /// An unrecognized token yields `None` so the caller can decide what a
    /// malformed row means (the engine skips the row).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "like" => Some(FilterOp::Like),
            "equals" => Some(FilterOp::Equals),
            "in" => Some(FilterOp::In),
            "gt" => Some(FilterOp::Gt),
            "gte" => Some(FilterOp::Gte),
            "lt" => Some(FilterOp::Lt),
            "lte" => Some(FilterOp::Lte),
            _ => None,
        }
    }
}

impl core::str::FromStr for FilterOp {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        FilterOp::from_str(s).ok_or(())
    }
}

impl core::fmt::Display for FilterOp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The indices of each OR group in a filter list.
///
/// This is the single definition of the grouping rule: the Desk's local
/// `apply_filters` and the engine's `ferris_document::filters` both evaluate a
/// row as "any group matches fully", and both used to re-derive the rule. A
/// filter carrying `or` ends the current group, except at the start, where
/// there is nothing to end (DOC5).
#[must_use]
pub fn or_groups(flags: impl IntoIterator<Item = bool>) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for (index, starts_new_group) in flags.into_iter().enumerate() {
        if starts_new_group && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        current.push(index);
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// Encode filters as the `[[field, op, value, or?], …]` rows the engine parses.
///
/// `ferris_document::parse_list_filters` is the reader for this exact grammar;
/// the two are kept in step by the round-trip test below.
pub fn filters_json(filters: &[Filter]) -> String {
    let rows: Vec<(String, String, String, bool)> = filters.iter().map(Filter::tuple).collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}

fn default_page_length() -> u64 {
    20
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Desk serializes the body of `POST /api/resource/{doctype}/list`, so
    /// a change here silently breaks every list request.
    #[test]
    fn only_liked_is_absent_unless_set() {
        let params = ListRequest {
            limit_page_length: 20,
            ..Default::default()
        };
        let body = serde_json::to_value(&params).expect("params serialize");
        assert!(body.get("only_liked").is_none());

        let filtered = ListRequest {
            limit_page_length: 20,
            only_liked: Some(true),
            ..Default::default()
        };
        let body = serde_json::to_value(&filtered).expect("params serialize");
        assert_eq!(body["only_liked"], serde_json::json!(true));
    }

    /// An absent page length must not mean "return the whole table".
    #[test]
    fn an_absent_page_length_defaults_to_twenty() {
        let parsed: ListRequest =
            serde_json::from_str(r#"{"limit_start":0}"#).expect("request parses");
        assert_eq!(parsed.limit_page_length, 20);
    }

    /// The filters grammar is a contract with `ferris_document::parse_list_filters`:
    /// a row is `[field, op, value]` plus an optional OR flag.
    #[test]
    fn filters_encode_as_the_rows_the_engine_parses() {
        let filters = vec![
            Filter {
                field: "status".into(),
                op: FilterOp::Equals,
                value: "open".into(),
                or: false,
            },
            Filter {
                field: "title".into(),
                op: FilterOp::Like,
                value: "urgent".into(),
                or: true,
            },
        ];
        assert_eq!(
            filters_json(&filters),
            r#"[["status","equals","open",false],["title","like","urgent",true]]"#
        );
    }

    /// `from_str` is case-insensitive because filter rows reach the client from
    /// saved view state, which is user-editable JSON.
    /// Every operator the UI offers must parse back to itself, otherwise
    /// saving a view state and reloading it silently changes the filter.
    #[test]
    fn every_offered_operator_round_trips_through_its_wire_token() {
        for (_, op) in FILTER_OPERATORS {
            assert_eq!(FilterOp::from_str(op.as_str()), Some(*op));
            assert_eq!(op.as_str().parse::<FilterOp>().ok(), Some(*op));
        }
        assert_eq!(FilterOp::from_str("gte"), Some(FilterOp::Gte));
        assert_eq!(FilterOp::from_str("GTE"), Some(FilterOp::Gte));
        assert_eq!(FilterOp::from_str("nonsense"), None);
    }
}

#[cfg(test)]
mod or_group_tests {
    use super::or_groups;

    #[test]
    fn an_or_flag_starts_a_new_group() {
        assert_eq!(or_groups([false, false]), vec![vec![0, 1]]);
        assert_eq!(or_groups([false, true, false]), vec![vec![0], vec![1, 2]]);
        // A leading `or` has no group to end.
        assert_eq!(or_groups([true, false]), vec![vec![0, 1]]);
        assert!(or_groups([]).is_empty());
    }
}
