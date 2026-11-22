//! The activity timeline of one document.
//!
//! Composes the side tables that already record history — `tab_version` (the
//! write log), `tab_comment` and `tab_like` — into the single, time-ordered
//! [`ActivityEntry`] feed the Desk renders. Nothing new is stored here: a
//! dedicated activity table would be a second source of truth for events that
//! the version/comment/like tables already own, and the two would drift.
//!
//! This lives in `ferris-document` (not `ferris-api`) because it reads the
//! stores directly and, for the "which fields changed" summary, reuses the
//! engine's own [`crate::engine::is_engine_field`] rule rather than restating it.

use std::collections::HashMap;

use ferris_core::{Result, Value};
use ferris_db::Db;

use crate::engagement::{CommentStore, LikeStore};
use crate::versioning::{ChangeType, VersionManager};

// Re-exported so `ferris-api` can build the feed's response type without also
// importing from `ferris-schema`.
pub use ferris_schema::contract::activity::{ActivityEntry, ActivityKind};

/// Read-only view over a document's activity sources.
#[derive(Clone)]
pub struct ActivityStore {
    db: Db,
}

impl ActivityStore {
    #[must_use]
    pub const fn new(db: Db) -> Self {
        Self { db }
    }

    /// Every recorded event for `doctype`/`name`, oldest first.
    ///
    /// The feed is a merge of three queries; each source is already sorted, so
    /// the merge only has to sort the small combined vector. Entries that share
    /// a timestamp keep a stable relative order (versions before comments before
    /// likes), because `sort_by` is stable and the sources are pushed in that
    /// order.
    ///
    /// # Errors
    ///
    /// Returns the storage error of any of the three source queries.
    pub async fn feed(&self, doctype: &str, name: &str) -> Result<Vec<ActivityEntry>> {
        let mut entries = Vec::new();

        // --- writes ---
        let versions = VersionManager::with_db(self.db.clone())
            .get_versions(doctype, name)
            .await?;
        let mut previous: Option<&HashMap<String, Value>> = None;
        for version in &versions {
            let kind = match version.change_type {
                ChangeType::Create => ActivityKind::Create,
                ChangeType::Update => ActivityKind::Update,
                ChangeType::Submit => ActivityKind::Submit,
                ChangeType::Cancel => ActivityKind::Cancel,
                ChangeType::Amend => ActivityKind::Amend,
                ChangeType::Delete => ActivityKind::Delete,
                // A restore re-writes the row; the feed has no separate kind,
                // so it reads as the write it is.
                ChangeType::Restore => ActivityKind::Update,
            };
            // Only an `update` has a meaningful field list; for the others the
            // kind alone says what happened.
            let fields = if kind == ActivityKind::Update {
                changed_fields(previous, &version.data)
            } else {
                Vec::new()
            };
            entries.push(ActivityEntry {
                kind,
                actor: version.changed_by.clone(),
                at: version.changed_at.to_rfc3339(),
                detail: version.comment.clone(),
                fields,
            });
            previous = Some(&version.data);
        }

        // --- comments ---
        for comment in CommentStore::new(self.db.clone())
            .list(doctype, name)
            .await?
        {
            entries.push(ActivityEntry {
                kind: if comment.is_reply() {
                    ActivityKind::Reply
                } else {
                    ActivityKind::Comment
                },
                actor: comment.commented_by,
                at: comment.created_at,
                detail: Some(comment.body),
                fields: Vec::new(),
            });
        }

        // --- likes ---
        for (actor, at) in LikeStore::new(self.db.clone())
            .likers(doctype, name)
            .await?
        {
            entries.push(ActivityEntry {
                kind: ActivityKind::Like,
                actor,
                at,
                detail: None,
                fields: Vec::new(),
            });
        }

        entries.sort_by(|a, b| a.at.cmp(&b.at));
        Ok(entries)
    }
}

/// Field names that differ between two snapshots, ignoring the engine-owned
/// fields (they change on every save and are not a user edit).
fn changed_fields(
    previous: Option<&HashMap<String, Value>>,
    current: &HashMap<String, Value>,
) -> Vec<String> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let mut fields: Vec<String> = previous
        .keys()
        .chain(current.keys())
        .filter(|key| !crate::engine::is_engine_field(key))
        .filter(|key| previous.get(*key) != current.get(*key))
        .cloned()
        .collect();
    fields.sort();
    fields.dedup();
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_core::Value;

    async fn db() -> Db {
        ferris_db::memory_db().await.unwrap()
    }

    fn data(pairs: &[(&str, &str)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), Value::from(*v)))
            .collect()
    }

    /// Engine fields never show up as "changed": `modified`/`modified_by` and
    /// `name` move on every save, so counting them would turn every activity row
    /// into "everything changed".
    #[test]
    fn changed_fields_ignores_engine_fields_and_reports_the_rest() {
        let before = data(&[
            ("title", "a"),
            ("qty", "1"),
            ("modified", "2026-01-01T00:00:00Z"),
            ("modified_by", "ada"),
        ]);
        let after = data(&[
            ("title", "b"),
            ("qty", "2"),
            ("modified", "2026-01-02T00:00:00Z"),
            ("modified_by", "grace"),
        ]);
        assert_eq!(
            changed_fields(Some(&before), &after),
            vec!["qty".to_string(), "title".to_string()]
        );
    }

    #[test]
    fn changed_fields_reports_added_and_removed() {
        let before = data(&[("title", "a"), ("old", "x")]);
        let after = data(&[("title", "a"), ("new", "y")]);
        assert_eq!(
            changed_fields(Some(&before), &after),
            vec!["new".to_string(), "old".to_string()]
        );
    }

    /// A create has no previous snapshot, so it reports no field list — the
    /// kind already says "created".
    #[test]
    fn changed_fields_without_a_previous_snapshot_is_empty() {
        assert!(changed_fields(None, &data(&[("title", "a")])).is_empty());
    }

    /// The feed merges writes, comments and likes in one ascending timeline.
    #[tokio::test]
    async fn feed_merges_writes_comments_and_likes_in_order() {
        let db = db().await;
        let store = ActivityStore::new(db.clone());

        let versions = VersionManager::with_db(db.clone());
        versions.init().await.unwrap();
        versions
            .create_version(
                "ToDo",
                "TD-1",
                data(&[("title", "call")]),
                "ada@local".to_string(),
                ChangeType::Create,
                None,
            )
            .await
            .unwrap();
        versions
            .create_version(
                "ToDo",
                "TD-1",
                data(&[("title", "call the client")]),
                "ada@local".to_string(),
                ChangeType::Update,
                None,
            )
            .await
            .unwrap();

        CommentStore::new(db.clone())
            .add("ToDo", "TD-1", "grace@local", "on it", None)
            .await
            .unwrap();
        LikeStore::new(db.clone())
            .toggle("ToDo", "TD-1", "bob@local")
            .await
            .unwrap();

        let feed = store.feed("ToDo", "TD-1").await.unwrap();
        let kinds: Vec<ActivityKind> = feed.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ActivityKind::Create,
                ActivityKind::Update,
                ActivityKind::Comment,
                ActivityKind::Like,
            ]
        );
        // Ascending timestamps.
        assert!(feed.windows(2).all(|w| w[0].at <= w[1].at));
        // The update carries its changed field, and only that.
        assert_eq!(feed[1].fields, vec!["title".to_string()]);
        assert_eq!(feed[2].detail.as_deref(), Some("on it"));
    }
}
