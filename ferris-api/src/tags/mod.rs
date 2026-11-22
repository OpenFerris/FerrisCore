//! Tags: the standard `tags` field, the `Tag` catalogue and the `Tag Link`
//! association registry.
//!
//! Every registered `DocType` carries a standard `tags` field
//! ([`ferris_schema::tags_field`]): a client-writable, comma-separated list of
//! labels. This module turns that flat list into the relational shape Frappe
//! uses — without asking the Desk to render a child-table editor it does not
//! have (see `ferris-web/src/components/views/form/layout.rs`, "edición de
//! tablas hijo aún pendiente"):
//!
//! * [`TAG_LINK`] — one row per `(document_type, document_name, tag)`, the
//!   association registry [`documents_with_tag`] queries;
//! * `Tag` (declared in [`crate::doctypes`]) — the catalogue of labels, one row
//!   per distinct tag, created on demand ("auto-alta").
//!
//! [`install`] mounts an engine hook on every registered `DocType` so the two
//! stay in step with the flat field on every save and lose the document's links
//! when it is deleted. The hook is best-effort by design: a tag problem must
//! never strand the document write that triggered it.
//!
//! The flat field is the source of truth and both `Tag` and `Tag Link` are
//! *derived* views of it, so a document remains readable even when the sync
//! could not run (no hook installed, a database hiccup): the tag labels are in
//! the document's own payload, not only in a side table.

#![allow(clippy::doc_markdown)]

use std::sync::Arc;

use ferris_core::hooks::{DocumentHook, HookContext, HookFn, HookPoint, HookRegistry};
use ferris_db::Db;
use ferris_schema::{DoctypeMeta, FieldMeta};

use crate::doctypes::{DESK_MODULE, TAG, system_manager};

mod service;
#[cfg(test)]
mod tests;

pub use service::{TagLink, documents_with_tag, parse_tags, remove_document, sync_document};

/// The association `DocType`: `(document_type, document_name, tag)`.
pub const TAG_LINK: &str = "Tag Link";

/// Tag under which the hooks this module installs are registered, so
/// re-installing replaces instead of stacking.
const HOOK_TAG: &str = "ferris-api-tags";

/// The points that matter. `OnUpdate` fires for a fresh insert as well as an
/// update (the engine mirrors Frappe there), so one point covers both; `OnTrash`
/// runs after a delete commits.
const POINTS: [HookPoint; 2] = [HookPoint::OnUpdate, HookPoint::OnTrash];

/// The `Tag Link` DocType declaration.
///
/// A child-table `DocType` (Frappe's shape) that is `hidden`: it is reached
/// through the standard `tags` field and [`documents_with_tag`], never as a
/// workspace link. The three fields are the association key; `tag` is a `Link`
/// into the `Tag` catalogue, whose document name *is* the label, so a link row
/// stays human-readable.
#[must_use]
pub fn tag_link_meta() -> DoctypeMeta {
    DoctypeMeta::new(TAG_LINK, DESK_MODULE)
        .description("An association between a document and a tag.")
        .title_field("tag")
        .sort("modified", "desc")
        .child_table()
        .hidden()
        .allow_delete()
        .field(
            FieldMeta::link("document_type", "DocType")
                .label("Document Type")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::data("document_name")
                .label("Document Name")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("tag", TAG)
                .label("Tag")
                .required()
                .in_list_view(),
        )
        .permission(system_manager())
}

/// Register the tag-support `DocType` (`Tag Link`) in the global registry.
///
/// Called from [`crate::doctypes::register_doctypes`] so the catalog stays the
/// single registration entry point; `Tag` itself is declared there.
pub fn register() {
    ferris_schema::register_doctype(tag_link_meta());
}

/// Shared state of the tag hook: the one pool every derived write runs against.
struct TagsContext {
    db: Db,
}

/// Mount the tag-sync hooks for every currently-registered `DocType`.
///
/// Idempotent: the `ferris-api-tags` tag makes a second call replace the first,
/// and re-running after plugins register new `DocType`s covers them without
/// stacking duplicates. Call after the full catalog is registered.
pub fn install(pool: Db) {
    let ctx = Arc::new(TagsContext { db: pool });
    for doctype in ferris_schema::list_doctypes() {
        // `Tag`/`Tag Link` are this hook's own outputs: the service refuses them
        // (it would otherwise reconcile their own standard `tags` field and
        // write associations under themselves), so there is nothing to mount.
        if doctype == TAG || doctype == TAG_LINK {
            continue;
        }
        for point in POINTS {
            HookRegistry::replace_global(
                HOOK_TAG,
                doctype.clone(),
                point,
                document_hook(ctx.clone(), point),
                0,
                None,
            );
        }
    }
}

fn document_hook(ctx: Arc<TagsContext>, point: HookPoint) -> HookFn {
    Box::new(
        move |_hook_ctx: &mut HookContext, doc: &mut dyn DocumentHook| {
            let ctx = ctx.clone();
            let doctype = doc.doctype().to_string();
            let name = doc.name().to_string();
            let tags = doc.get_field(ferris_schema::TAGS_FIELD).cloned();
            Box::pin(async move {
                // `Tag`/`Tag Link` are excluded at registration time
                // (`install`), and the service refuses them anyway: this hook is
                // a thin adapter with no policy of its own.
                let outcome = match point {
                    HookPoint::OnTrash => remove_document(&ctx.db, &doctype, &name).await,
                    _ => sync_document(&ctx.db, &doctype, &name, tags.as_ref()).await,
                };
                if let Err(error) = outcome {
                    // Best-effort: a tag problem must not strand the save.
                    tracing::warn!(%doctype, %name, %error, "tag sync failed");
                }
                Ok(())
            })
        },
    )
}
