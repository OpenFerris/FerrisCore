//! The activity timeline of a single document.
//!
//! `GET /api/activity/{doctype}/{name}` returns a flat, time-ordered list of
//! [`ActivityEntry`]: the document's write log (`tab_version`), its comments and
//! its likes, merged into the one feed the Desk renders as a vertical timeline.
//!
//! There is no `Activity` table: the feed is a *composition* of side tables that
//! already exist, built at read time. A separate log would be a second source of
//! truth for the same events, guaranteed to drift from the tables it summarises.

use serde::{Deserialize, Serialize};

/// What happened.
///
/// Serialised `snake_case` so the wire value reads the same as the module that
/// produced it and a new variant cannot silently reuse an existing spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    /// The document was created.
    Create,
    /// A field changed.
    Update,
    /// The document was submitted.
    Submit,
    /// The document was cancelled.
    Cancel,
    /// A submitted/cancelled document was amended into a new draft.
    Amend,
    /// The document was deleted.
    Delete,
    /// A top-level comment was posted.
    Comment,
    /// A reply to a comment was posted.
    Reply,
    /// A user liked the document.
    Like,
}

/// One row of the timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub kind: ActivityKind,
    /// Who caused the entry (user id).
    pub actor: String,
    /// RFC 3339 timestamp; the feed is ordered by this.
    pub at: String,
    /// Free text: the comment body, a version's note, or `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// For an `update`, the field names that changed. Empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire spelling is part of the contract: a rename would break every
    /// stored/consumed feed, so pin it.
    #[test]
    fn kinds_serialise_snake_case() {
        let cases = [
            (ActivityKind::Create, "\"create\""),
            (ActivityKind::Update, "\"update\""),
            (ActivityKind::Submit, "\"submit\""),
            (ActivityKind::Cancel, "\"cancel\""),
            (ActivityKind::Amend, "\"amend\""),
            (ActivityKind::Delete, "\"delete\""),
            (ActivityKind::Comment, "\"comment\""),
            (ActivityKind::Reply, "\"reply\""),
            (ActivityKind::Like, "\"like\""),
        ];
        for (kind, expected) in cases {
            assert_eq!(serde_json::to_string(&kind).unwrap(), expected);
        }
    }

    /// An entry with neither detail nor fields still round-trips: those two
    /// fields are omitted, not required, so the Desk's `Option`/`Vec` defaults
    /// are what an older server's body relies on.
    #[test]
    fn empty_optionals_are_omitted() {
        let entry = ActivityEntry {
            kind: ActivityKind::Like,
            actor: "ada@example.com".to_string(),
            at: "2026-01-01T00:00:00Z".to_string(),
            detail: None,
            fields: Vec::new(),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "like",
                "actor": "ada@example.com",
                "at": "2026-01-01T00:00:00Z",
            })
        );
        let parsed: ActivityEntry = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, entry);
    }
}
