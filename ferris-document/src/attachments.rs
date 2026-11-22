//! Document attachments (`tab_attachment`).
//!
//! The row is a *registry* entry, not the bytes: it stores where a file lives
//! (`file_url`) plus its display metadata, so the Desk can list, link and
//! delete attachments through the JSON contract without streaming binary over
//! it. The bytes themselves are the job of `ferris-files`; this table only
//! records which of them belong to which document.
//!
//! Side table keyed by document, like [`crate::engagement`]'s `tab_comment`, so
//! any `DocType` gets attachments with no schema change.

use crate::runtime::now_rfc3339;
use ferris_core::{Result, ValidationError, Value};
use ferris_db::{Db, Row, Stmt, sql::Column};

pub use ferris_schema::contract::collaboration::Attachment;

const TAB_ATTACHMENT: &str = "tab_attachment";

fn attachment_table() -> Vec<Column> {
    vec![
        Column::new("id", "VARCHAR(255)").not_null().primary_key(),
        Column::new("doctype", "VARCHAR(255)").not_null(),
        Column::new("docname", "VARCHAR(255)").not_null(),
        Column::new("file_name", "VARCHAR(512)").not_null(),
        Column::new("file_url", "VARCHAR(1024)").not_null(),
        Column::new("file_size", "INTEGER").not_null().default("0"),
        Column::new("uploaded_by", "VARCHAR(255)").not_null(),
        Column::new("created_at", "VARCHAR(64)").not_null(),
    ]
}

fn row_to_attachment(row: &Row) -> Attachment {
    Attachment {
        id: row.str("id").unwrap_or_default(),
        doctype: row.str("doctype").unwrap_or_default(),
        docname: row.str("docname").unwrap_or_default(),
        file_name: row.str("file_name").unwrap_or_default(),
        file_url: row.str("file_url").unwrap_or_default(),
        // A stored size is never negative; clamp before widening so a corrupt
        // row cannot wrap into a huge `u64`.
        file_size: row.opt_i64("file_size").unwrap_or(0).max(0).cast_unsigned(),
        uploaded_by: row.str("uploaded_by").unwrap_or_default(),
        created_at: row.opt_str("created_at").unwrap_or_default(),
    }
}

/// Attachments per document, for any `DocType`.
#[derive(Clone)]
pub struct AttachmentStore {
    db: Db,
}

impl AttachmentStore {
    #[must_use]
    pub const fn new(db: Db) -> Self {
        Self { db }
    }

    /// Create the backing table and its index (idempotent).
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table or the index.
    pub async fn init(&self) -> Result<()> {
        self.db
            .ensure_table(TAB_ATTACHMENT, &attachment_table())
            .await?;
        // Every read filters on (doctype, docname).
        self.db
            .ensure_index(
                "idx_attachment_doc",
                TAB_ATTACHMENT,
                &["doctype", "docname"],
                false,
            )
            .await
    }

    /// Register an already-stored file as an attachment of the document.
    ///
    /// `file_url` is required and validated at this edge: an attachment whose
    /// URL is empty would render as a link to nothing, so it is rejected here
    /// rather than silently listed.
    ///
    /// # Errors
    ///
    /// Returns a validation error for an empty file name or URL, and the
    /// storage error of creating the table or inserting the row.
    pub async fn add(
        &self,
        doctype: &str,
        docname: &str,
        uploaded_by: &str,
        file_name: &str,
        file_url: &str,
        file_size: u64,
    ) -> Result<Attachment> {
        let file_name = file_name.trim();
        let file_url = file_url.trim();
        if file_name.is_empty() {
            return Err(ferris_core::Error::Validation(ValidationError::Field {
                field: "file_name".to_string(),
                message: "an attachment needs a file name".to_string(),
            }));
        }
        if file_url.is_empty() {
            return Err(ferris_core::Error::Validation(ValidationError::Field {
                field: "file_url".to_string(),
                message: "an attachment needs a file URL".to_string(),
            }));
        }

        self.init().await?;
        let attachment = Attachment {
            id: uuid::Uuid::new_v4().to_string(),
            doctype: doctype.to_string(),
            docname: docname.to_string(),
            file_name: file_name.to_string(),
            file_url: file_url.to_string(),
            file_size,
            uploaded_by: uploaded_by.to_string(),
            created_at: now_rfc3339(),
        };
        self.db
            .execute(&Stmt::new(
                format!(
                    "INSERT INTO {TAB_ATTACHMENT} \
                     (id, doctype, docname, file_name, file_url, file_size, uploaded_by, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
                ),
                vec![
                    Value::from(attachment.id.as_str()),
                    Value::from(attachment.doctype.as_str()),
                    Value::from(attachment.docname.as_str()),
                    Value::from(attachment.file_name.as_str()),
                    Value::from(attachment.file_url.as_str()),
                    Value::from(attachment.file_size.cast_signed()),
                    Value::from(attachment.uploaded_by.as_str()),
                    Value::from(attachment.created_at.as_str()),
                ],
            ))
            .await?;
        Ok(attachment)
    }

    /// Remove the attachment `id` from the document.
    ///
    /// The delete is scoped by `(doctype, docname, id)`, so an `id` from
    /// another document cannot be used to remove an attachment that is not on
    /// this one. Removing an id that is not there deletes nothing and succeeds.
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table or deleting the row.
    pub async fn remove(&self, doctype: &str, docname: &str, id: &str) -> Result<()> {
        self.init().await?;
        self.db
            .execute(&Stmt::new(
                format!(
                    "DELETE FROM {TAB_ATTACHMENT} \
                     WHERE doctype = ? AND docname = ? AND id = ?"
                ),
                vec![Value::from(doctype), Value::from(docname), Value::from(id)],
            ))
            .await?;
        Ok(())
    }

    /// Every attachment of a document, oldest first.
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table or reading the rows.
    pub async fn list(&self, doctype: &str, docname: &str) -> Result<Vec<Attachment>> {
        self.init().await?;
        let rows = self
            .db
            .fetch_all(&Stmt::new(
                format!(
                    "SELECT * FROM {TAB_ATTACHMENT} \
                     WHERE doctype = ? AND docname = ? ORDER BY created_at ASC"
                ),
                vec![Value::from(doctype), Value::from(docname)],
            ))
            .await?;
        Ok(rows.iter().map(row_to_attachment).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> AttachmentStore {
        AttachmentStore::new(ferris_db::memory_db().await.unwrap())
    }

    #[tokio::test]
    async fn add_and_list_round_trip() {
        let store = store().await;
        let added = store
            .add(
                "ToDo",
                "TD-1",
                "ada@example.com",
                "notes.txt",
                "/files/notes.txt",
                42,
            )
            .await
            .unwrap();
        assert_eq!(added.file_size, 42);

        let listed = store.list("ToDo", "TD-1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].file_name, "notes.txt");
    }

    #[tokio::test]
    async fn remove_is_scoped_to_the_document() {
        let store = store().await;
        let a = store
            .add("ToDo", "TD-1", "ada@example.com", "a.txt", "/files/a", 1)
            .await
            .unwrap();
        store
            .add("ToDo", "TD-2", "ada@example.com", "b.txt", "/files/b", 1)
            .await
            .unwrap();

        // The id belongs to TD-1, so asking to delete it from TD-2 must not
        // touch TD-1's row.
        store.remove("ToDo", "TD-2", &a.id).await.unwrap();
        assert_eq!(store.list("ToDo", "TD-1").await.unwrap().len(), 1);

        store.remove("ToDo", "TD-1", &a.id).await.unwrap();
        assert!(store.list("ToDo", "TD-1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn add_rejects_an_empty_url() {
        let store = store().await;
        assert!(
            store
                .add("ToDo", "TD-1", "ada@example.com", "x.txt", "  ", 0)
                .await
                .is_err()
        );
    }
}
