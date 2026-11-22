use ferris_core::{Result, Value};
use ferris_db::Db;
use ferris_document::DocumentEngine;

use super::meta::context;
use super::{DocStore, effective_meta, validate_doctype, validate_name};

impl DocStore {
    /// Rename a document, gated by the DocType's `allow_rename`.
    ///
    /// Both names go through [`validate_name`]: the new primary key becomes a
    /// route parameter immediately, so it is adjudicated before it reaches the
    /// engine.
    pub async fn rename(pool: &Db, doctype: &str, name: &str, new_name: &str) -> Result<Value> {
        validate_doctype(doctype)?;
        validate_name(name)?;
        validate_name(new_name)?;
        let meta = effective_meta(pool, doctype).await?;
        DocumentEngine::new(pool.clone())
            .rename(&context(doctype, "", meta.as_ref()), name, new_name)
            .await
    }

    /// Transition a document's docstatus (submit/cancel).
    pub async fn set_docstatus(
        pool: &Db,
        doctype: &str,
        name: &str,
        target: ferris_document::DocStatus,
    ) -> Result<Value> {
        validate_doctype(doctype)?;
        validate_name(name)?;
        let meta = effective_meta(pool, doctype).await?;
        let naming = doctype.to_uppercase();
        let ctx = context(doctype, &naming, meta.as_ref());
        let value = DocumentEngine::new(pool.clone())
            .set_docstatus(&ctx, name, target)
            .await?;
        match &target {
            ferris_document::DocStatus::Submitted => {
                crate::notify::notify_doc_submit(doctype, name, &value);
            }
            ferris_document::DocStatus::Cancelled => {
                crate::notify::notify_doc_cancel(doctype, name, &value);
            }
            ferris_document::DocStatus::Draft => {}
        }
        Ok(value)
    }

    /// Amend a submitted/cancelled document: clone it into a fresh draft with
    /// a new name (the engine re-runs naming, validation and lifecycle hooks).
    ///
    /// Requires `Permission::Amend` on the **source** document, not just
    /// `Read`. This used to read the source with no check at all, so
    /// `POST /api/resource/<DocType>/<name>/amend` both disclosed and copied any
    /// document; and once `Read` was enforced it would still have let any
    /// principal who can *see* a submitted document duplicate it, because the
    /// owner shortcut in `ferris_auth::permissions` grants `Amend` on the basis
    /// of ownership while a plain reader has no such claim. Frappe lists
    /// `amend` among the per-`DocType` rights (`frappe/permissions.py:22`).
    pub async fn amend(pool: &Db, doctype: &str, name: &str) -> Result<Value> {
        validate_doctype(doctype)?;
        validate_name(name)?;
        let meta = effective_meta(pool, doctype).await?;
        let naming_series = meta
            .as_ref()
            .map(|m| m.naming_series.clone())
            .unwrap_or_default();
        let ctx = context(doctype, &naming_series, meta.as_ref());
        DocumentEngine::new(pool.clone()).amend(&ctx, name).await
    }

    /// Restore a soft-deleted document from `Deleted Document`.
    ///
    /// `restore_token` is the trash row's primary key (returned by the engine's
    /// `deleted_documents` reader); the document is reinserted under its
    /// original name only while that name is still free, and the engine appends
    /// a `Restore` version so the history explains the reappearance.
    pub async fn restore(pool: &Db, doctype: &str, restore_token: &str) -> Result<Value> {
        validate_doctype(doctype)?;
        let meta = effective_meta(pool, doctype).await?;
        DocumentEngine::new(pool.clone())
            .restore(&context(doctype, "", meta.as_ref()), restore_token)
            .await
    }
}
