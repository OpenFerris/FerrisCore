//! Per-document collaboration side tables: assignments, attachments and shares.
//!
//! Like [`super::engagement`], these are the wire contract rather than a copy of
//! the stores' own structs: `ferris-document` produces them directly, the
//! `ferris-api` handlers serialize them, and the Desk deserializes the very same
//! types, so a field rename is a compile error on both sides instead of a
//! runtime `null`.
//!
//! Timestamps are strings (RFC 3339), not `chrono` values, because this module
//! compiles with `default-features = false` for the browser and must not pull
//! the date stack into the hydrate graph. The action that *writes* a timestamp
//! lives with the store that owns the table.

use serde::{Deserialize, Serialize};

/// A user assigned to a document (`tab_assignment`).
///
/// The key is `(doctype, docname, assigned_to)`, so assigning the same user
/// twice is a property of the schema rather than something the caller must
/// check first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub id: String,
    pub doctype: String,
    pub docname: String,
    /// The user the document is assigned to.
    pub assigned_to: String,
    /// Who made the assignment.
    pub assigned_by: String,
    pub created_at: String,
}

/// A file attached to a document (`tab_attachment`).
///
/// The row is a *registry* entry, not the bytes: `file_url` points at where the
/// file actually lives (an object-store URL, or a `ferris-files` download
/// path), which is how the Desk lists, links and later deletes it without the
/// API having to stream binary through the JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub doctype: String,
    pub docname: String,
    /// The display name (the original file name, extension included).
    pub file_name: String,
    /// Where the bytes live.
    pub file_url: String,
    /// Size in bytes; `0` when the uploader did not report one.
    #[serde(default)]
    pub file_size: u64,
    pub uploaded_by: String,
    pub created_at: String,
}

/// A share grant on a document (`tab_share`).
///
/// A mirror of `ferris_document::sharing::DocShare` with string timestamps, so
/// the contract module stays free of `chrono` while the store keeps the
/// `chrono` values it needs to update `modified_at`.
// These four booleans are not four flags of one concept: `read`/`write`/`share`
// are the three independent ACL levels a grant can carry and `everyone` says
// whom it targets. Renaming them into a narrower representation would break the
// wire contract the store, the API and the Desk all serialize against.
#[allow(
    clippy::struct_excessive_bools,
    reason = "the three permission levels are separate dimensions of one ACL grant"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    pub name: String,
    pub doctype: String,
    pub docname: String,
    /// The user the document is shared with.
    pub user: String,
    pub read: bool,
    pub write: bool,
    pub share: bool,
    /// Whether the grant applies to everyone rather than to `user` alone.
    pub everyone: bool,
    pub created_at: String,
    pub modified_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An attachment without a reported size must still parse: the size is
    /// optional information, not part of the identity of the row.
    #[test]
    fn attachment_size_defaults_to_zero() {
        let attachment: Attachment = serde_json::from_value(serde_json::json!({
            "id": "a1",
            "doctype": "ToDo",
            "docname": "TD-1",
            "file_name": "notes.txt",
            "file_url": "/files/a1",
            "uploaded_by": "ada@example.com",
            "created_at": "2026-10-17T00:00:00Z"
        }))
        .expect("attachment parses");
        assert_eq!(attachment.file_size, 0);
    }

    /// The engagement-page round trip proves the field names the Desk reads are
    /// the ones the server writes.
    #[test]
    fn share_round_trips_through_json() {
        let share = Share {
            name: "s1".to_string(),
            doctype: "ToDo".to_string(),
            docname: "TD-1".to_string(),
            user: "grace@example.com".to_string(),
            read: true,
            write: false,
            share: false,
            everyone: false,
            created_at: "2026-10-17T00:00:00Z".to_string(),
            modified_at: "2026-10-17T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&share).expect("share serializes");
        let back: Share = serde_json::from_str(&json).expect("share parses");
        assert_eq!(back, share);
    }
}
