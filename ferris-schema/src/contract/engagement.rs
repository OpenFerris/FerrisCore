//! Per-document engagement: likes, comment counts and the comments themselves.
//!
//! These double as the engine's own return types (`ferris_document`'s
//! `LikeStore`/`CommentStore` produce them directly), so there is exactly one
//! definition of "a page of engagement state" rather than one in the store and
//! a copy in the client that has to be re-mapped field by field.
//!
//! [`DocComment`] lives here, not in `ferris-document`, for the same reason:
//! it crosses the wire on `GET`/`POST /api/engagement/{doctype}/{name}/comments`,
//! so the store and the Desk must agree on its shape without either depending
//! on the other.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Result of a like toggle (`POST /api/engagement/{doctype}/{name}/like`).
///
/// It reports the state that *resulted*, so the caller never has to guess
/// whether the toggle landed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LikeState {
    pub liked: bool,
    pub count: u64,
}

/// Everything the list view needs about one page of documents, in a single
/// read: which of them the user liked, how many likes each has, and how many
/// comments each has (`POST /api/engagement/{doctype}`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EngagementPage {
    /// Docnames the user has liked.
    #[serde(default)]
    pub liked: HashSet<String>,
    /// Like count per docname.
    #[serde(default)]
    pub like_counts: HashMap<String, u64>,
    /// Comment count per docname.
    #[serde(default)]
    pub comment_counts: HashMap<String, u64>,
}

impl EngagementPage {
    pub fn is_liked(&self, name: &str) -> bool {
        self.liked.contains(name)
    }

    pub fn like_count(&self, name: &str) -> u64 {
        self.like_counts.get(name).copied().unwrap_or(0)
    }

    pub fn comment_count(&self, name: &str) -> u64 {
        self.comment_counts.get(name).copied().unwrap_or(0)
    }
}

/// A comment on a document (`tab_comment`), as returned by the comments
/// endpoints and rendered by the form's discussion panel.
///
/// `parent` is the id of the comment this one replies to, or `None` for a
/// top-level comment. Modelled as a nullable column rather than a nested tree
/// so a reply stays an ordinary row (queryable, orderable, pageable) and the
/// Desk nests it at render time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocComment {
    pub id: String,
    pub doctype: String,
    pub docname: String,
    pub commented_by: String,
    pub body: String,
    pub created_at: String,
    /// The parent comment's `id`, or `None` for a top-level comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

impl DocComment {
    /// Whether this comment is a reply to another comment.
    #[must_use]
    pub fn is_reply(&self) -> bool {
        self.parent.as_deref().is_some_and(|p| !p.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row the server said nothing about must read as "not liked, no counts",
    /// never as a panic or a stale value.
    #[test]
    fn an_absent_document_reads_as_zero() {
        let page = EngagementPage::default();
        assert!(!page.is_liked("TD-1"));
        assert_eq!(page.like_count("TD-1"), 0);
        assert_eq!(page.comment_count("TD-1"), 0);
    }

    /// A page with no likes is still a valid response, so the maps must default
    /// rather than fail to deserialize.
    #[test]
    fn empty_engagement_page_still_parses() {
        let page: EngagementPage =
            serde_json::from_value(serde_json::json!({ "liked": [] })).expect("page parses");
        assert!(page.liked.is_empty());
        assert_eq!(page.like_count("TD-1"), 0);
    }
}
