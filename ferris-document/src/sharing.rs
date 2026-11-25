//! Document sharing

use ferris_core::{Error, Result, Value};
use ferris_db::{
    Db, Row, Stmt,
    sql::{Column, UpsertBuilder, UpsertSet},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Authorization hook: whether an actor may manage shares for a document.
#[async_trait::async_trait]
pub trait ShareAuthorizer: Send + Sync {
    async fn can_share(&self, doctype: &str, docname: &str, actor: &str) -> bool;
}

/// Share permissions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SharePermission {
    Read,
    Write,
    Share,
    All,
}

impl SharePermission {
    /// Case-insensitive parse. Retained as the single parse entry point; the
    /// canonical trait impl is [`FromStr`] below (DOC11 — the inherent `from_str`
    /// used to shadow `std::str::FromStr`, which clippy flags as
    /// `should_implement_trait`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "share" => Some(Self::Share),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

impl std::str::FromStr for SharePermission {
    type Err = Error;

    /// DOC11: the same case-insensitive parse, exposed through the trait so a
    /// caller writes `s.parse::<SharePermission>()` and gets an error instead of
    /// `None` for an unknown name.
    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s).ok_or_else(|| {
            Error::Validation(ferris_core::ValidationError::Invalid(format!(
                "unknown share permission: {s}"
            )))
        })
    }
}

/// Document share record
///
/// DOC15: the four bools are four independent `tab_share` columns, each with its
/// own `0`/`1` value in the database and its own name in the JSON contract.
/// Collapsing them into a bitset would change the persisted row shape and the
/// wire format for no gain, so the lint is wrong here.
#[allow(clippy::struct_excessive_bools)] // one bool per independent permission column
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocShare {
    pub name: String,
    pub doctype: String,
    pub docname: String,
    pub user: String,
    pub read: bool,
    pub write: bool,
    pub share: bool,
    pub everyone: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub modified_at: chrono::DateTime<chrono::Utc>,
}

impl DocShare {
    pub fn new(doctype: String, docname: String, user: String) -> Self {
        let now = chrono::Utc::now();
        Self {
            name: uuid::Uuid::new_v4().to_string(),
            doctype,
            docname,
            user,
            read: false,
            write: false,
            share: false,
            everyone: false,
            created_at: now,
            modified_at: now,
        }
    }

    pub fn set_permission(&mut self, perm: SharePermission, value: bool) {
        match perm {
            SharePermission::Read => self.read = value,
            SharePermission::Write => self.write = value,
            SharePermission::Share => self.share = value,
            SharePermission::All => {
                self.read = value;
                self.write = value;
                self.share = value;
            }
        }
        self.modified_at = chrono::Utc::now();
    }

    pub fn has_permission(&self, perm: SharePermission) -> bool {
        match perm {
            SharePermission::Read => self.read,
            SharePermission::Write => self.write,
            SharePermission::Share => self.share,
            SharePermission::All => self.read && self.write && self.share,
        }
    }
}

pub(crate) const TAB_SHARE: &str = "tab_share";

/// `everyone` is a 0/1 integer rather than a dialect boolean: SQLite has no
/// boolean type, MySQL's `BOOLEAN` is a synonym for `TINYINT(1)`, and
/// [`Row::opt_bool`] reads back whichever the backend produced.
pub(crate) fn share_table() -> Vec<Column> {
    vec![
        Column::new("name", "VARCHAR(255)").not_null().primary_key(),
        Column::new("doctype", "VARCHAR(255)").not_null(),
        Column::new("docname", "VARCHAR(255)").not_null(),
        Column::new("shared_with", "VARCHAR(255)").not_null(),
        Column::new("can_read", "INTEGER").not_null().default("0"),
        Column::new("can_write", "INTEGER").not_null().default("0"),
        Column::new("can_share", "INTEGER").not_null().default("0"),
        Column::new("everyone", "INTEGER").not_null().default("0"),
        Column::new("created_at", "VARCHAR(64)").not_null(),
        Column::new("modified_at", "VARCHAR(64)").not_null(),
    ]
}

fn row_to_share(row: &Row) -> DocShare {
    let parse = |s: Option<String>| {
        s.and_then(|v| chrono::DateTime::parse_from_rfc3339(&v).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now)
    };
    DocShare {
        name: row.str("name").unwrap_or_default(),
        doctype: row.str("doctype").unwrap_or_default(),
        docname: row.str("docname").unwrap_or_default(),
        user: row.str("shared_with").unwrap_or_default(),
        read: row.opt_bool("can_read").unwrap_or(false),
        write: row.opt_bool("can_write").unwrap_or(false),
        share: row.opt_bool("can_share").unwrap_or(false),
        everyone: row.opt_bool("everyone").unwrap_or(false),
        created_at: parse(row.opt_str("created_at")),
        modified_at: parse(row.opt_str("modified_at")),
    }
}

/// Share manager for document sharing.
///
/// Backed by `tab_share` when a database is configured (survives restarts and is
/// shared across instances); otherwise falls back to an in-memory map. When a
/// [`ShareAuthorizer`] is set, granting a share requires it to approve the
/// acting user.
pub struct ShareManager {
    shares: parking_lot::RwLock<HashMap<String, Vec<DocShare>>>,
    db: Option<Db>,
    authorizer: Option<Arc<dyn ShareAuthorizer>>,
}

impl ShareManager {
    pub fn new() -> Self {
        Self {
            shares: parking_lot::RwLock::new(HashMap::new()),
            db: None,
            authorizer: None,
        }
    }

    pub fn with_db(db: Db) -> Self {
        Self {
            shares: parking_lot::RwLock::new(HashMap::new()),
            db: Some(db),
            authorizer: None,
        }
    }

    pub fn with_authorizer(mut self, authorizer: Arc<dyn ShareAuthorizer>) -> Self {
        self.authorizer = Some(authorizer);
        self
    }

    /// Create the backing table (no-op without a database).
    pub async fn init(&self) -> Result<()> {
        let Some(db) = &self.db else {
            return Ok(());
        };
        db.ensure_table(TAB_SHARE, &share_table()).await?;
        // Every read path filters on (doctype, docname).
        db.ensure_index("idx_share_doc", TAB_SHARE, &["doctype", "docname"], false)
            .await
    }

    /// Share a document with a user, authorized by `granted_by`.
    pub async fn share(
        &self,
        doctype: &str,
        docname: &str,
        user: &str,
        permissions: Vec<SharePermission>,
        granted_by: &str,
    ) -> Result<DocShare> {
        if let Some(authorizer) = &self.authorizer {
            if !authorizer.can_share(doctype, docname, granted_by).await {
                return Err(Error::Authz(ferris_core::AuthzError::PermissionDenied {
                    action: "share".to_string(),
                    resource: format!("{doctype}/{docname}"),
                }));
            }
        }

        let mut share = DocShare::new(doctype.to_string(), docname.to_string(), user.to_string());
        for perm in permissions {
            share.set_permission(perm, true);
        }

        if let Some(db) = &self.db {
            let read = Value::from(i64::from(share.read));
            let write = Value::from(i64::from(share.write));
            let can_share = Value::from(i64::from(share.share));
            let everyone = Value::from(i64::from(share.everyone));
            let modified_at = Value::from(share.modified_at.to_rfc3339());
            db.upsert(
                UpsertBuilder::new(db.dialect(), TAB_SHARE)
                    .values(vec![
                        ("name", Value::from(share.name.as_str())),
                        ("doctype", Value::from(share.doctype.as_str())),
                        ("docname", Value::from(share.docname.as_str())),
                        ("shared_with", Value::from(share.user.as_str())),
                        ("can_read", read.clone()),
                        ("can_write", write.clone()),
                        ("can_share", can_share.clone()),
                        ("everyone", everyone.clone()),
                        ("created_at", Value::from(share.created_at.to_rfc3339())),
                        ("modified_at", modified_at.clone()),
                    ])
                    .conflict_on(["name"])
                    .on_update("can_read", UpsertSet::Value(read))
                    .on_update("can_write", UpsertSet::Value(write))
                    .on_update("can_share", UpsertSet::Value(can_share))
                    .on_update("everyone", UpsertSet::Value(everyone))
                    .on_update("modified_at", UpsertSet::Value(modified_at))
                    .build()?,
            )
            .await?;
        } else {
            let key = format!("{doctype}/{docname}");
            self.shares
                .write()
                .entry(key)
                .or_default()
                .push(share.clone());
        }

        Ok(share)
    }

    /// Unshare a document from a user
    pub async fn unshare(&self, doctype: &str, docname: &str, user: &str) -> Result<()> {
        if let Some(db) = &self.db {
            db.execute(&Stmt::new(
                format!(
                    "DELETE FROM {TAB_SHARE} WHERE doctype = ? AND docname = ? AND shared_with = ?"
                ),
                vec![
                    Value::from(doctype),
                    Value::from(docname),
                    Value::from(user),
                ],
            ))
            .await?;
            return Ok(());
        }

        let key = format!("{doctype}/{docname}");
        if let Some(doc_shares) = self.shares.write().get_mut(&key) {
            doc_shares.retain(|s| s.user != user);
        }
        Ok(())
    }

    /// Get all shares for a document
    pub async fn get_shares(&self, doctype: &str, docname: &str) -> Result<Vec<DocShare>> {
        if let Some(db) = &self.db {
            let rows = db
                .fetch_all(&Stmt::new(
                    format!("SELECT * FROM {TAB_SHARE} WHERE doctype = ? AND docname = ?"),
                    vec![Value::from(doctype), Value::from(docname)],
                ))
                .await?;
            return Ok(rows.iter().map(row_to_share).collect());
        }

        let key = format!("{doctype}/{docname}");
        Ok(self.shares.read().get(&key).cloned().unwrap_or_default())
    }

    /// Check if user has permission on document.
    ///
    /// Honours `everyone` shares in addition to explicit per-user grants.
    pub async fn check_permission(
        &self,
        doctype: &str,
        docname: &str,
        user: &str,
        perm: SharePermission,
    ) -> Result<bool> {
        for share in self.get_shares(doctype, docname).await? {
            if (share.user == user || share.everyone) && share.has_permission(perm) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The docnames within `doctype` that `user` may read through a share,
    /// explicit or `everyone`.
    ///
    /// This is the list-view counterpart of [`Self::check_permission`]: a caller
    /// paging a DocType needs the whole set of shared documents up front, because
    /// the permission condition there is folded into one query rather than asked
    /// per row (`DocumentEngine::list`). A share that grants `Write`/`Share`
    /// without `Read` is not included — reading is what the list needs.
    ///
    /// Returns a set even when no database is configured, built from the
    /// in-memory shares, so the contract is identical with and without a backend.
    pub async fn readable_docnames(
        &self,
        doctype: &str,
        user: &str,
    ) -> Result<std::collections::HashSet<String>> {
        let shares = if let Some(db) = &self.db {
            // Narrow to the rows that can matter before decoding them: this user's
            // own shares plus the ones granted to everyone. A site can hold many
            // shares for a DocType, and the list path should not decode all of
            // them on every page.
            let rows = db
                .fetch_all(&Stmt::new(
                    format!(
                        "SELECT * FROM {TAB_SHARE} \
                         WHERE doctype = ? AND (shared_with = ? OR everyone <> 0)"
                    ),
                    vec![Value::from(doctype), Value::from(user)],
                ))
                .await?;
            rows.iter().map(row_to_share).collect::<Vec<_>>()
        } else {
            // The in-memory map is keyed `doctype/docname`; take every entry for
            // this doctype. A linear scan is fine: this is a per-list call, not a
            // per-row one, and the map is the fallback path only.
            let prefix = format!("{doctype}/");
            self.shares
                .read()
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix))
                .flat_map(|(_, shares)| shares.clone())
                .collect()
        };

        Ok(shares
            .into_iter()
            .filter(|share| {
                (share.user == user || share.everyone)
                    && share.has_permission(SharePermission::Read)
            })
            .map(|share| share.docname)
            .collect())
    }
}

impl Default for ShareManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_share_manager() {
        let manager = ShareManager::new();

        let share = manager
            .share(
                "Test",
                "doc-1",
                "user1",
                vec![SharePermission::Read, SharePermission::Write],
                "admin",
            )
            .await
            .unwrap();
        assert!(share.read);
        assert!(share.write);

        let shares = manager.get_shares("Test", "doc-1").await.unwrap();
        assert_eq!(shares.len(), 1);

        let has_perm = manager
            .check_permission("Test", "doc-1", "user1", SharePermission::Read)
            .await
            .unwrap();
        assert!(has_perm);

        manager.unshare("Test", "doc-1", "user1").await.unwrap();
        let shares = manager.get_shares("Test", "doc-1").await.unwrap();
        assert_eq!(shares.len(), 0);
    }
}
