//! Per-document engagement by user: likes (`tab_like`) and comments
//! (`tab_comment`).
//!
//! Both live in side tables keyed by document rather than in a column on the
//! document, the same shape as [`crate::sharing`]'s `tab_share`. A like belongs
//! to one user and a comment to one author, so neither can be a per-doctype
//! field without one user's data showing up in another's view. Side tables
//! also mean any `DocType` gets both with no schema change.

use std::collections::{HashMap, HashSet};

use crate::runtime::now_rfc3339;
use ferris_core::{Error, Result, ValidationError, Value};
use ferris_db::{Db, Row, Stmt, sql::Column};

// The payloads below are the wire contract: `LikeStore::toggle` returns
// `LikeState` straight to the `POST /api/engagement/.../like` handler,
// `read_page` returns `EngagementPage` to the list endpoint, and
// `CommentStore::{add,list}` exchange `DocComment` with the comments endpoints.
// They are defined once, in `ferris-schema`, and re-exported here so callers
// that already import from this module keep working. Re-declaring them here
// would mean the Desk had to re-map the response field by field, which is where
// such drift starts.
pub use ferris_schema::contract::engagement::{DocComment, EngagementPage, LikeState};

const TAB_LIKE: &str = "tab_like";
const TAB_COMMENT: &str = "tab_comment";

/// The key is `(doctype, docname, liked_by)`, which is what makes one like per
/// user per document a property of the schema rather than of the code.
fn like_table() -> Vec<Column> {
    vec![
        Column::new("doctype", "VARCHAR(255)")
            .not_null()
            .primary_key(),
        Column::new("docname", "VARCHAR(255)")
            .not_null()
            .primary_key(),
        Column::new("liked_by", "VARCHAR(255)")
            .not_null()
            .primary_key(),
        Column::new("created_at", "VARCHAR(64)").not_null(),
    ]
}

fn comment_table() -> Vec<Column> {
    vec![
        Column::new("id", "VARCHAR(255)").not_null().primary_key(),
        Column::new("doctype", "VARCHAR(255)").not_null(),
        Column::new("docname", "VARCHAR(255)").not_null(),
        Column::new("commented_by", "VARCHAR(255)").not_null(),
        Column::new("body", "TEXT").not_null(),
        Column::new("created_at", "VARCHAR(64)").not_null(),
        // The parent comment's `id`, or NULL for a top-level comment. A column
        // (not a nested row) so replies stay ordinary, orderable rows.
        Column::new("parent", "VARCHAR(255)"),
    ]
}

/// Likes per document, for any `DocType`.
#[derive(Clone)]
pub struct LikeStore {
    db: Db,
}

impl LikeStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Create the backing table (idempotent).
    pub async fn init(&self) -> Result<()> {
        self.db.ensure_table(TAB_LIKE, &like_table()).await?;
        // `liked_names` and `like_counts` both filter on doctype alone.
        self.db
            .ensure_index(
                "idx_like_doctype_user",
                TAB_LIKE,
                &["doctype", "liked_by"],
                false,
            )
            .await
    }

    /// Like the document if it is not liked yet, unlike it otherwise, and
    /// report the state that resulted.
    ///
    /// The delete decides the toggle: a row removed means it was liked, and a
    /// row that was not there means it was not. That keeps the two steps a
    /// single round trip and needs no read first.
    pub async fn toggle(&self, doctype: &str, docname: &str, user: &str) -> Result<LikeState> {
        self.init().await?;

        let removed = self
            .db
            .execute(&Stmt::new(
                format!(
                    "DELETE FROM {TAB_LIKE} WHERE doctype = ? AND docname = ? AND liked_by = ?"
                ),
                vec![
                    Value::from(doctype),
                    Value::from(docname),
                    Value::from(user),
                ],
            ))
            .await?;

        let liked = removed.is_empty();
        if liked {
            self.db
                .execute(&Stmt::new(
                    format!(
                        "INSERT INTO {TAB_LIKE} (doctype, docname, liked_by, created_at) \
                         VALUES (?, ?, ?, ?)"
                    ),
                    vec![
                        Value::from(doctype),
                        Value::from(docname),
                        Value::from(user),
                        Value::from(now_rfc3339()),
                    ],
                ))
                .await?;
        }

        let count = self
            .like_counts(doctype)
            .await?
            .get(docname)
            .copied()
            .unwrap_or(0);

        Ok(LikeState { liked, count })
    }

    /// Docnames of `doctype` that `user` has liked.
    pub async fn liked_names(&self, doctype: &str, user: &str) -> Result<HashSet<String>> {
        self.init().await?;
        let rows = self
            .db
            .fetch_all(&Stmt::new(
                format!("SELECT docname FROM {TAB_LIKE} WHERE doctype = ? AND liked_by = ?"),
                vec![Value::from(doctype), Value::from(user)],
            ))
            .await?;
        Ok(rows.iter().filter_map(|r| r.str("docname").ok()).collect())
    }

    /// Everyone who liked the document, with the time they did, oldest first.
    ///
    /// `like_counts` answers "how many", which is all the list badge needs; the
    /// activity feed needs *who*, so this is the per-document detail of the same
    /// table.
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table or reading the rows.
    pub async fn likers(&self, doctype: &str, docname: &str) -> Result<Vec<(String, String)>> {
        self.init().await?;
        let rows = self
            .db
            .fetch_all(&Stmt::new(
                format!(
                    "SELECT liked_by, created_at FROM {TAB_LIKE} \
                     WHERE doctype = ? AND docname = ? ORDER BY created_at ASC"
                ),
                vec![Value::from(doctype), Value::from(docname)],
            ))
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.str("liked_by").unwrap_or_default(),
                    r.opt_str("created_at").unwrap_or_default(),
                )
            })
            .collect())
    }

    /// Like count per docname of `doctype`.
    pub async fn like_counts(&self, doctype: &str) -> Result<HashMap<String, u64>> {
        self.init().await?;
        let rows = self
            .db
            .fetch_all(&Stmt::new(
                format!(
                    "SELECT docname, COUNT(*) AS c FROM {TAB_LIKE} WHERE doctype = ? GROUP BY docname"
                ),
                vec![Value::from(doctype)],
            ))
            .await?;
        Ok(counts_by_docname(&rows))
    }
}

/// Decode a `tab_comment` row. `parent` is nullable, so an absent value reads
/// back as `None` rather than the empty string.
fn row_to_comment(row: &Row) -> DocComment {
    DocComment {
        id: row.str("id").unwrap_or_default(),
        doctype: row.str("doctype").unwrap_or_default(),
        docname: row.str("docname").unwrap_or_default(),
        commented_by: row.str("commented_by").unwrap_or_default(),
        body: row.str("body").unwrap_or_default(),
        created_at: row.opt_str("created_at").unwrap_or_default(),
        parent: row.opt_str("parent").filter(|p| !p.is_empty()),
    }
}

/// Comments per document, for any `DocType`.
#[derive(Clone)]
pub struct CommentStore {
    db: Db,
}

impl CommentStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Create the backing table and its index (idempotent).
    ///
    /// `ensure_table` only creates; a database that already had `tab_comment`
    /// before replies existed needs the `parent` column added, otherwise every
    /// read would fail with "no such column". `ensure_column` tolerates the
    /// duplicate-column error, so both a fresh and an upgraded database end up
    /// with the same schema.
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table, its column or the index.
    pub async fn init(&self) -> Result<()> {
        self.db.ensure_table(TAB_COMMENT, &comment_table()).await?;
        self.db
            .ensure_column(TAB_COMMENT, &Column::new("parent", "VARCHAR(255)"))
            .await?;
        self.db
            .ensure_index(
                "idx_comment_doc",
                TAB_COMMENT,
                &["doctype", "docname"],
                false,
            )
            .await
    }

    /// Post a comment, optionally as a reply to `parent`.
    ///
    /// An empty or whitespace-only body is rejected here, not in the caller, so
    /// every path that comments validates the same way. A `parent` that does
    /// not name an existing comment of the *same* document is rejected too: a
    /// reply pointing at another document (or at nothing) would render as an
    /// orphan the discussion panel could never show.
    ///
    /// # Errors
    ///
    /// Returns a validation error for an empty, oversized or unknown `parent`
    /// comment, and the storage error of creating the table or inserting.
    pub async fn add(
        &self,
        doctype: &str,
        docname: &str,
        user: &str,
        body: &str,
        parent: Option<&str>,
    ) -> Result<DocComment> {
        let body = body.trim();
        if body.is_empty() {
            return Err(Error::Validation(ValidationError::Invalid(
                "Comment body must not be empty".to_string(),
            )));
        }
        if body.len() > 5000 {
            return Err(Error::Validation(ValidationError::Invalid(
                "Comment body must be at most 5000 characters".to_string(),
            )));
        }

        self.init().await?;

        let parent = parent.map(str::trim).filter(|p| !p.is_empty());
        if let Some(parent) = parent
            && !self.exists(doctype, docname, parent).await?
        {
            return Err(Error::Validation(ValidationError::Field {
                field: "parent".to_string(),
                message: format!("El comentario {parent} no existe en este documento"),
            }));
        }

        let comment = DocComment {
            id: uuid::Uuid::new_v4().to_string(),
            doctype: doctype.to_string(),
            docname: docname.to_string(),
            commented_by: user.to_string(),
            body: body.to_string(),
            created_at: now_rfc3339(),
            parent: parent.map(str::to_string),
        };

        self.db
            .execute(&Stmt::new(
                format!(
                    "INSERT INTO {TAB_COMMENT} \
                     (id, doctype, docname, commented_by, body, created_at, parent) \
                     VALUES (?, ?, ?, ?, ?, ?, ?)"
                ),
                vec![
                    Value::from(comment.id.as_str()),
                    Value::from(comment.doctype.as_str()),
                    Value::from(comment.docname.as_str()),
                    Value::from(comment.commented_by.as_str()),
                    Value::from(comment.body.as_str()),
                    Value::from(comment.created_at.as_str()),
                    comment.parent.as_deref().map_or(Value::Null, Value::from),
                ],
            ))
            .await?;

        Ok(comment)
    }

    /// Every comment on a document, oldest first.
    ///
    /// Ordered by `created_at`, whose RFC 3339 spelling sorts
    /// chronologically; the discussion panel re-threads replies by `parent`,
    /// so a flat, time-ordered list is exactly what it needs.
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table or reading the rows.
    pub async fn list(&self, doctype: &str, docname: &str) -> Result<Vec<DocComment>> {
        self.init().await?;
        let rows = self
            .db
            .fetch_all(&Stmt::new(
                format!(
                    "SELECT * FROM {TAB_COMMENT} \
                     WHERE doctype = ? AND docname = ? ORDER BY created_at ASC"
                ),
                vec![Value::from(doctype), Value::from(docname)],
            ))
            .await?;
        Ok(rows.iter().map(row_to_comment).collect())
    }

    /// Whether `id` is a comment of this document — the parent check replies
    /// run before insert.
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table or reading the row.
    pub async fn exists(&self, doctype: &str, docname: &str, id: &str) -> Result<bool> {
        self.init().await?;
        let row = self
            .db
            .fetch_optional(&Stmt::new(
                format!(
                    "SELECT 1 FROM {TAB_COMMENT} \
                     WHERE doctype = ? AND docname = ? AND id = ?"
                ),
                vec![Value::from(doctype), Value::from(docname), Value::from(id)],
            ))
            .await?;
        Ok(row.is_some())
    }

    /// Comment count per docname of `doctype`.
    pub async fn counts(&self, doctype: &str) -> Result<HashMap<String, u64>> {
        self.init().await?;
        let rows = self
            .db
            .fetch_all(&Stmt::new(
                format!(
                    "SELECT docname, COUNT(*) AS c FROM {TAB_COMMENT} \
                     WHERE doctype = ? GROUP BY docname"
                ),
                vec![Value::from(doctype)],
            ))
            .await?;
        Ok(counts_by_docname(&rows))
    }
}

/// `{docname: COUNT(*)}` from a `GROUP BY docname` result.
fn counts_by_docname(rows: &[Row]) -> HashMap<String, u64> {
    rows.iter()
        .filter_map(|row| Some((row.str("docname").ok()?, row.count("c"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Db {
        ferris_db::memory_db().await.unwrap()
    }

    #[tokio::test]
    async fn toggling_adds_then_removes_the_like() {
        let store = LikeStore::new(db().await);

        let first = store
            .toggle("ToDo", "TD-0001", "ada@example.com")
            .await
            .unwrap();
        assert!(first.liked);
        assert_eq!(first.count, 1);

        let second = store
            .toggle("ToDo", "TD-0001", "ada@example.com")
            .await
            .unwrap();
        assert!(!second.liked);
        assert_eq!(second.count, 0);
    }

    #[tokio::test]
    async fn likes_are_counted_per_user_and_scoped_to_the_doctype() {
        let store = LikeStore::new(db().await);
        store
            .toggle("ToDo", "TD-0001", "ada@example.com")
            .await
            .unwrap();
        store
            .toggle("ToDo", "TD-0001", "grace@example.com")
            .await
            .unwrap();
        store
            .toggle("ToDo", "TD-0002", "ada@example.com")
            .await
            .unwrap();
        store
            .toggle("Note", "NT-0001", "ada@example.com")
            .await
            .unwrap();

        let counts = store.like_counts("ToDo").await.unwrap();
        assert_eq!(counts.get("TD-0001"), Some(&2));
        assert_eq!(counts.get("TD-0002"), Some(&1));
        assert!(!counts.contains_key("NT-0001"));

        let liked = store.liked_names("ToDo", "ada@example.com").await.unwrap();
        assert_eq!(
            liked,
            HashSet::from(["TD-0001".to_string(), "TD-0002".to_string()])
        );
        assert!(
            store
                .liked_names("Note", "ada@example.com")
                .await
                .unwrap()
                .contains("NT-0001")
        );
    }

    #[tokio::test]
    async fn comments_are_counted_per_document() {
        let store = CommentStore::new(db().await);
        store
            .add("ToDo", "TD-0001", "ada@example.com", "primero", None)
            .await
            .unwrap();
        store
            .add("ToDo", "TD-0001", "grace@example.com", "segundo", None)
            .await
            .unwrap();
        store
            .add("ToDo", "TD-0002", "ada@example.com", "otro", None)
            .await
            .unwrap();

        let counts = store.counts("ToDo").await.unwrap();
        assert_eq!(counts.get("TD-0001"), Some(&2));
        assert_eq!(counts.get("TD-0002"), Some(&1));
    }

    #[tokio::test]
    async fn a_blank_comment_is_rejected() {
        let store = CommentStore::new(db().await);
        let err = store
            .add("ToDo", "TD-0001", "ada@example.com", "   \n ", None)
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            Error::Validation(ValidationError::Invalid(_))
        ));
        assert!(store.counts("ToDo").await.unwrap().is_empty());
    }

    /// A reply points at a real comment of the *same* document, and `list`
    /// returns it with its parent set so the panel can thread it.
    #[tokio::test]
    async fn a_reply_records_its_parent() {
        let store = CommentStore::new(db().await);
        let root = store
            .add("ToDo", "TD-0001", "ada@example.com", "¿lo revisamos?", None)
            .await
            .unwrap();

        let reply = store
            .add("ToDo", "TD-0001", "grace@example.com", "sí", Some(&root.id))
            .await
            .unwrap();
        assert!(reply.is_reply());
        assert_eq!(reply.parent.as_deref(), Some(root.id.as_str()));

        let all = store.list("ToDo", "TD-0001").await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(!all[0].is_reply());
        assert_eq!(all[1].parent.as_deref(), Some(root.id.as_str()));
    }

    /// A reply cannot point at nothing, and cannot borrow a comment id from a
    /// different document — both would render as orphans.
    #[tokio::test]
    async fn a_reply_must_reference_a_comment_of_the_same_document() {
        let store = CommentStore::new(db().await);
        let other = store
            .add("ToDo", "TD-0002", "ada@example.com", "otro doc", None)
            .await
            .unwrap();

        let missing = store
            .add("ToDo", "TD-0001", "ada@example.com", "hola", Some("nope"))
            .await;
        assert!(matches!(
            missing,
            Err(Error::Validation(ValidationError::Field { .. }))
        ));

        let cross_doc = store
            .add(
                "ToDo",
                "TD-0001",
                "ada@example.com",
                "hola",
                Some(&other.id),
            )
            .await;
        assert!(cross_doc.is_err());
    }
}
