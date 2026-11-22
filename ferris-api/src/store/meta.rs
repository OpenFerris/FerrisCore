use ferris_auth::Permission;
use ferris_core::{Error, Result};
use ferris_db::Db;
use ferris_document::{DocumentEngine, EngineContext};
use ferris_schema::DoctypeMeta;

use super::DocStore;

/// Effective metadata for `doctype`: registry metadata plus runtime
/// customizations. `None` for Doctypes that have no compiled definition.
pub async fn effective_meta(pool: &Db, doctype: &str) -> Result<Option<DoctypeMeta>> {
    ferris_schema::effective_meta(pool, doctype).await
}

/// Whether document `name` exists under `ctx`, distinguishing a genuine miss
/// from every other failure.
///
/// The upsert sites used to write `engine.get(ctx, name).await.is_ok()`, which
/// reads a permission error or a storage failure as "no row yet" and falls into
/// the `create` branch — turning a denied read into a spurious `AlreadyExists`
/// conflict, or a transient storage error into a duplicate insert. Only
/// `DocumentError::NotFound` means "no row"; everything else propagates so the
/// caller can answer it.
pub(crate) async fn document_exists(
    engine: &DocumentEngine,
    ctx: &EngineContext<'_>,
    name: &str,
) -> Result<bool> {
    match engine.get(ctx, name).await {
        Ok(_) => Ok(true),
        Err(Error::Document(ferris_core::DocumentError::NotFound { .. })) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Refuse the request unless the ambient caller may `Read` the document.
///
/// The engagement/activity endpoints act on side tables (`tab_comment`,
/// `tab_like`, `tab_version`), so without this they answered for **any**
/// document id an authenticated caller could name — the same IDOR shape as the
/// old resource reads, one layer over: a caller who cannot read a
/// document could still read its whole discussion and history, or comment on it.
///
/// Reading through [`DocStore::get`] reuses the engine's single permission
/// policy rather than restating it, so "who may see a document" is decided in
/// exactly one place. The count-aggregation endpoint
/// (`POST /api/engagement/{doctype}`) is deliberately *not* guarded this way:
/// it answers for a whole page at once, and a per-document check there would be
/// an N+1 on the list path for a difference of counts only.
pub(crate) async fn ensure_readable(pool: &Db, doctype: &str, name: &str) -> Result<()> {
    DocStore::get(pool, doctype, name).await.map(|_| ())
}

/// Write gate for a document's collaboration side tables (assignments,
/// attachments, shares).
///
/// Mutating who a document is assigned to, what is attached to it, or who it is
/// shared with is a write on the document, so it requires `Permission::Write`,
/// not merely `Read`. Like [`ensure_readable`], the policy is delegated to the
/// engine (`DocumentEngine::get_for`) rather than restated here, so there is
/// exactly one definition of "who may write a document".
///
/// Public so the SSR bridge (`ferris-web`'s `DocStoreApi`) can apply the very
/// same gate in-process; SSR and hydrate must not disagree about who may write.
///
/// # Errors
///
/// Returns the engine's authorization error when the caller lacks `Write`, or a
/// metadata/storage error while resolving the document's permissions.
pub async fn ensure_writable(pool: &Db, doctype: &str, name: &str) -> Result<()> {
    let meta = effective_meta(pool, doctype).await?;
    let ctx = context(doctype, "", meta.as_ref());
    DocumentEngine::new(pool.clone())
        .get_for(&ctx, name, Permission::Write)
        .await
        .map(|_| ())
}

/// Require `Permission::Share` on the document before mutating its share list.
///
/// Sharing is its own DocType right in Frappe (`has_permission(..., "share")`),
/// not an implication of `Write`: a caller allowed to edit a document must not
/// be able to hand it to somebody else, and a caller allowed to share must not
/// be able to edit it. Like [`ensure_writable`], the decision is delegated to
/// the engine so there is one definition of the rule (shares included).
///
/// # Errors
///
/// Returns the engine's authorization error when the caller lacks `Share`, or a
/// metadata/storage error while resolving the document's permissions.
pub async fn ensure_shareable(pool: &Db, doctype: &str, name: &str) -> Result<()> {
    let meta = effective_meta(pool, doctype).await?;
    let ctx = context(doctype, "", meta.as_ref());
    DocumentEngine::new(pool.clone())
        .get_for(&ctx, name, Permission::Share)
        .await
        .map(|_| ())
}

/// Append one row to a document's write log (`tab_version`), which the activity
/// timeline (`GET /api/activity/{doctype}/{name}`) reads back.
///
/// **Removed from this layer.** The engine records the row inside the write's
/// own transaction, exactly once, for every surface (REST, typed `crud`, plugin
/// `docstore`). Keeping a second writer here would produce two rows per write
/// and let the two disagree about what was stored. `VersionManager` remains the
/// reader.
pub(super) fn context<'a>(
    doctype: &str,
    naming_series: &'a str,
    meta: Option<&'a DoctypeMeta>,
) -> EngineContext<'a> {
    let mut ctx = EngineContext::new(doctype, naming_series);
    if let Some(meta) = meta {
        // The doctype's naming declaration is authoritative; `EngineContext::new`
        // only resolved the implicit series case. The Desk path and the typed
        // `crud` path read the same two fields.
        ctx.naming_type = meta.effective_naming_type();
        ctx.naming_field = meta.naming_field.clone();
        ctx.meta = Some(meta);
    }
    // Authorise REST writes as the ambient task user. API handlers are expected
    // to run under `ferris_document::with_user` (or an explicit service-context)
    // so unauthenticated writes are rejected by the engine (fails closed).
    ctx.user = ferris_document::current_user();
    // Unit tests pin document names (`P-1`, `TODO-1`, …) to make assertions
    // deterministic. That is the same trusted-fixture case as typed `crud`, not
    // a client request; production keeps the DOC-16 refusal.
    #[cfg(test)]
    {
        ctx.allow_explicit_name = true;
    }
    ctx
}
