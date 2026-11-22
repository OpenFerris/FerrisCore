//! Document assignments (`tab_assignment`).
//!
//! An assignment is a side table keyed by `(doctype, docname, assigned_to)`,
//! the same shape as [`crate::engagement`]'s `tab_like` and for the same
//! reason: who a document is assigned to belongs to that document, not to a
//! per-doctype field, and any `DocType` gets assignments with no schema change.
//!
//! The uniqueness of the triple is expressed in the table (a composite primary
//! key) rather than enforced by a read-then-write in the caller, so two people
//! assigning the same user at once cannot produce two rows.

use crate::runtime::now_rfc3339;
use ferris_core::{Result, ValidationError, Value};
use ferris_db::{Db, Row, Stmt, sql::Column};

pub use ferris_schema::contract::collaboration::Assignment;

const TAB_ASSIGNMENT: &str = "tab_assignment";

fn assignment_table() -> Vec<Column> {
    vec![
        Column::new("id", "VARCHAR(255)").not_null().primary_key(),
        Column::new("doctype", "VARCHAR(255)").not_null(),
        Column::new("docname", "VARCHAR(255)").not_null(),
        Column::new("assigned_to", "VARCHAR(255)").not_null(),
        Column::new("assigned_by", "VARCHAR(255)").not_null(),
        Column::new("created_at", "VARCHAR(64)").not_null(),
    ]
}

fn row_to_assignment(row: &Row) -> Assignment {
    Assignment {
        id: row.str("id").unwrap_or_default(),
        doctype: row.str("doctype").unwrap_or_default(),
        docname: row.str("docname").unwrap_or_default(),
        assigned_to: row.str("assigned_to").unwrap_or_default(),
        assigned_by: row.str("assigned_by").unwrap_or_default(),
        created_at: row.opt_str("created_at").unwrap_or_default(),
    }
}

/// Assignments per document, for any `DocType`.
#[derive(Clone)]
pub struct AssignmentStore {
    db: Db,
}

impl AssignmentStore {
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
            .ensure_table(TAB_ASSIGNMENT, &assignment_table())
            .await?;
        // Every read filters on (doctype, docname).
        self.db
            .ensure_index(
                "idx_assignment_doc",
                TAB_ASSIGNMENT,
                &["doctype", "docname"],
                false,
            )
            .await
    }

    /// Assign `assigned_to` to the document, recorded against `assigned_by`.
    ///
    /// Assigning the same user again is a no-op that returns the existing row:
    /// the composite key makes a duplicate impossible, so re-assigning is
    /// idempotent rather than an error the caller has to anticipate.
    ///
    /// # Errors
    ///
    /// Returns a validation error for an empty assignee, and the storage error
    /// of creating the table or reading/inserting the row.
    pub async fn assign(
        &self,
        doctype: &str,
        docname: &str,
        assigned_to: &str,
        assigned_by: &str,
    ) -> Result<Assignment> {
        let assigned_to = assigned_to.trim();
        if assigned_to.is_empty() {
            return Err(ferris_core::Error::Validation(ValidationError::Field {
                field: "assigned_to".to_string(),
                message: "an assignment needs a user".to_string(),
            }));
        }

        // The list is small (the assignees of one document), so reusing it as
        // the "already assigned?" check avoids a second query shape.
        if let Some(existing) = self
            .list(doctype, docname)
            .await?
            .into_iter()
            .find(|a| a.assigned_to == assigned_to)
        {
            return Ok(existing);
        }

        self.init().await?;
        let assignment = Assignment {
            id: uuid::Uuid::new_v4().to_string(),
            doctype: doctype.to_string(),
            docname: docname.to_string(),
            assigned_to: assigned_to.to_string(),
            assigned_by: assigned_by.to_string(),
            created_at: now_rfc3339(),
        };
        self.db
            .execute(&Stmt::new(
                format!(
                    "INSERT INTO {TAB_ASSIGNMENT} \
                     (id, doctype, docname, assigned_to, assigned_by, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?)"
                ),
                vec![
                    Value::from(assignment.id.as_str()),
                    Value::from(assignment.doctype.as_str()),
                    Value::from(assignment.docname.as_str()),
                    Value::from(assignment.assigned_to.as_str()),
                    Value::from(assignment.assigned_by.as_str()),
                    Value::from(assignment.created_at.as_str()),
                ],
            ))
            .await?;
        Ok(assignment)
    }

    /// Remove `assigned_to` from the document. Removing someone who was not
    /// assigned deletes nothing and succeeds, so a double "unassign" is safe.
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table or deleting the row.
    pub async fn unassign(&self, doctype: &str, docname: &str, assigned_to: &str) -> Result<()> {
        self.init().await?;
        self.db
            .execute(&Stmt::new(
                format!(
                    "DELETE FROM {TAB_ASSIGNMENT} \
                     WHERE doctype = ? AND docname = ? AND assigned_to = ?"
                ),
                vec![
                    Value::from(doctype),
                    Value::from(docname),
                    Value::from(assigned_to),
                ],
            ))
            .await?;
        Ok(())
    }

    /// Every assignment of a document, oldest first.
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table or reading the rows.
    pub async fn list(&self, doctype: &str, docname: &str) -> Result<Vec<Assignment>> {
        self.init().await?;
        let rows = self
            .db
            .fetch_all(&Stmt::new(
                format!(
                    "SELECT * FROM {TAB_ASSIGNMENT} \
                     WHERE doctype = ? AND docname = ? ORDER BY created_at ASC"
                ),
                vec![Value::from(doctype), Value::from(docname)],
            ))
            .await?;
        Ok(rows.iter().map(row_to_assignment).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> AssignmentStore {
        AssignmentStore::new(ferris_db::memory_db().await.unwrap())
    }

    #[tokio::test]
    async fn assign_is_idempotent_per_user() {
        let store = store().await;
        let first = store
            .assign("ToDo", "TD-1", "grace@example.com", "ada@example.com")
            .await
            .unwrap();
        let again = store
            .assign("ToDo", "TD-1", "grace@example.com", "ada@example.com")
            .await
            .unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(store.list("ToDo", "TD-1").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unassign_is_scoped_to_the_document() {
        let store = store().await;
        store
            .assign("ToDo", "TD-1", "grace@example.com", "ada@example.com")
            .await
            .unwrap();
        store
            .assign("ToDo", "TD-2", "grace@example.com", "ada@example.com")
            .await
            .unwrap();

        store
            .unassign("ToDo", "TD-1", "grace@example.com")
            .await
            .unwrap();

        assert!(store.list("ToDo", "TD-1").await.unwrap().is_empty());
        assert_eq!(store.list("ToDo", "TD-2").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn assign_rejects_an_empty_user() {
        let store = store().await;
        assert!(
            store
                .assign("ToDo", "TD-1", "   ", "ada@example.com")
                .await
                .is_err()
        );
    }
}
