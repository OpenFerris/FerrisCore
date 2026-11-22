use super::super::support::*;
use super::super::*;
use ferris_core::hooks::{DocStatus, HookPoint};
use ferris_core::permissions::Permission;
use ferris_core::{DocumentError, Error, Result, ValidationError, Value};
use ferris_db::Stmt;

// Tombstone storage, restore, purge and retention live one level down so this
// file stays about the delete lifecycle.
#[path = "delete_trash.rs"]
mod trash;

use self::trash::{legal_hold_error, legal_hold_from_env};

impl DocumentEngine {
    /// Delete a document by name.
    ///
    /// A submitted record is the tracked truth of a transaction and is refused
    /// until the workflow cancels it (Frappe: "Submitted Record cannot be
    /// deleted. You must Cancel it first."). Rows that reference
    /// `(doctype, docname)` — share, comment, like, assignment, attachment —
    /// are cascaded in the same transaction.
    ///
    /// SEC-07/DX-01: a **draft** is deleted hard, as before. A **cancelled**
    /// document is moved to `Deleted Document` with its full snapshot and its
    /// `tab_version` chain is kept, so cancelling and deleting can no longer
    /// make a booked invoice and its history disappear. [`Self::restore`]
    /// brings it back; [`Self::purge`] is the explicit, audited hard delete.
    pub async fn delete(&self, ctx: &EngineContext<'_>, name: &str) -> Result<()> {
        self.delete_with(ctx, name, None).await
    }

    /// [`Self::delete`] with a reason recorded on the tombstone.
    pub async fn delete_with(
        &self,
        ctx: &EngineContext<'_>,
        name: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        // A Virtual DocType owns its storage: the provider deletes the row, the
        // engine only authorizes the action. No `tab…` table is touched.
        if is_virtual(ctx) {
            let provider = virtual_doctype_provider(&ctx.doctype).ok_or_else(|| {
                Error::Document(ferris_core::DocumentError::VirtualDoctypeProviderMissing {
                    doctype: ctx.doctype.clone(),
                })
            })?;
            self.authorize(
                ctx,
                name,
                None,
                ferris_core::permissions::Permission::Delete,
            )
            .await?;
            return provider.delete(ctx, name);
        }
        if is_single(ctx) {
            return Err(Error::Document(
                ferris_core::DocumentError::SingleNotDeletable {
                    doctype: ctx.doctype.clone(),
                },
            ));
        }
        if legal_hold_from_env() {
            return Err(legal_hold_error());
        }
        let table = self.ensure_table(&ctx.doctype).await?;
        self.ensure_runtime_tables().await?;
        // The side tables are created lazily, so only the ones that exist are
        // cleaned. Discovered before the transaction so a batch cascades the
        // same set for every op.
        let side_tables = self.existing_side_tables().await?;

        let mut tx = self.db.begin().await?;
        match self
            .delete_via_with(
                &mut WriteSink::Tx(&mut tx),
                &table,
                ctx,
                name,
                &side_tables,
                reason,
            )
            .await
        {
            Ok(mut existing) => {
                tx.commit().await?;
                run_hooks(ctx, name, &mut existing, false, HookPoint::OnTrash).await?;
                Ok(())
            }
            Err(e) => {
                // Nothing was committed; unwind before surfacing the reason.
                tx.rollback().await.ok();
                Err(e)
            }
        }
    }

    /// [`Self::delete`] through an explicit [`WriteSink`] (`apply`'s open
    /// transaction, or the pool). Returns the deleted document so the caller
    /// can run the `OnTrash` hook after the transaction commits.
    pub(super) async fn delete_via(
        &self,
        sink: &mut WriteSink<'_>,
        table: &str,
        ctx: &EngineContext<'_>,
        name: &str,
        side_tables: &[String],
    ) -> Result<Value> {
        self.delete_via_with(sink, table, ctx, name, side_tables, None)
            .await
    }

    async fn delete_via_with(
        &self,
        sink: &mut WriteSink<'_>,
        table: &str,
        ctx: &EngineContext<'_>,
        name: &str,
        side_tables: &[String],
        reason: Option<&str>,
    ) -> Result<Value> {
        if is_single(ctx) {
            return Err(Error::Document(
                ferris_core::DocumentError::SingleNotDeletable {
                    doctype: ctx.doctype.clone(),
                },
            ));
        }
        if legal_hold_from_env() {
            return Err(legal_hold_error());
        }
        let (mut existing, owner_column) = read_row_via(sink, &ctx.doctype, name).await?;

        let status = existing
            .get("docstatus")
            .and_then(|v| v.as_str())
            .map(docstatus_from_str)
            .unwrap_or(DocStatus::Draft);
        // Frappe `delete_doc` rejects docstatus == 1 with this exact message;
        // draft and cancelled are deletable.
        if status == DocStatus::Submitted {
            return Err(Error::Validation(ValidationError::Invalid(
                "Submitted Record cannot be deleted. You must Cancel it first.".to_string(),
            )));
        }

        self.authorize(
            ctx,
            name,
            authz_owner(ctx, owner_column.as_deref()),
            Permission::Delete,
        )
        .await?;
        run_hooks(ctx, name, &mut existing, false, HookPoint::BeforeDelete).await?;

        // Only a cancelled document is booked evidence; its snapshot goes to
        // the trash before the live row disappears.
        if status == DocStatus::Cancelled {
            let data_json = canonical_blob(&existing)?;
            self.insert_tombstone_via(sink, ctx, name, &data_json, reason)
                .await?;
        }

        let affected = sink
            .execute(&Stmt::new(
                format!("DELETE FROM {} WHERE name = ?", q(sink.dialect(), table)?),
                vec![Value::from(name)],
            ))
            .await?;
        if affected.is_empty() {
            return Err(Error::Document(DocumentError::NotFound {
                doctype: ctx.doctype.clone(),
                name: name.to_string(),
            }));
        }
        for side in side_tables {
            // The cancelled document's history is the audit trail the tombstone
            // exists for: keep `tab_version` and append the delete marker.
            if status == DocStatus::Cancelled && side == crate::versioning::TAB_VERSION {
                continue;
            }
            sink.execute(&Stmt::new(
                format!(
                    "DELETE FROM {} WHERE doctype = ? AND docname = ?",
                    q(sink.dialect(), side)?
                ),
                vec![Value::from(ctx.doctype.clone()), Value::from(name)],
            ))
            .await?;
        }
        self.record_version(
            sink,
            ctx,
            name,
            &Value::Object(ferris_core::Map::new()),
            crate::versioning::ChangeType::Delete,
        )
        .await?;
        Ok(existing)
    }

    /// The lazily-created cascade tables that currently exist, keyed on
    /// `(doctype, docname)`.
    pub(super) async fn existing_side_tables(&self) -> Result<Vec<String>> {
        self.ensure_deleted_document_schema().await?;
        let mut side_tables = Vec::new();
        for side in [
            "tab_share",
            "tab_version",
            "tab_comment",
            "tab_like",
            "tab_assignment",
            "tab_attachment",
        ] {
            if self.db.table_exists(side).await? {
                side_tables.push(side.to_string());
            }
        }
        Ok(side_tables)
    }
}
