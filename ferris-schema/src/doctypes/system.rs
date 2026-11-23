//! System DocTypes owned by the document engine.
//!
//! `Deleted Document` is the trash: one row per soft-deleted document, written
//! by the engine when a **cancelled** document is deleted and read by
//! `DocumentEngine::restore`. The complete snapshot lives in `data`; the
//! `tab_version` chain is kept next to it, so the trash and the history
//! together preserve the evidence.
//!
//! Registration is pending in `doctypes/all.rs`/`doctypes/mod.rs` (outside this
//! change's allowed file set). The engine creates the physical table and drives
//! the lifecycle directly, so the behaviour does not depend on Desk metadata;
//! this declaration is the canonical field list for when the DocType is
//! registered.

use crate::registry::{DoctypeMeta, FieldMeta};

use super::consts::BUILD_MODULE;
use super::permissions::system_manager;

/// A deleted document that can still be restored.
#[must_use]
pub fn deleted_document_meta() -> DoctypeMeta {
    DoctypeMeta::new("Deleted Document", BUILD_MODULE)
        .description("A cancelled document kept for audit and restore.")
        .title_field("docname")
        .sort("deleted_at", "desc")
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(FieldMeta::data("doctype").label("DocType").in_list_view())
        .field(
            FieldMeta::data("docname")
                .label("Document Name")
                .in_list_view(),
        )
        .field(FieldMeta::text("data").label("Snapshot"))
        .field(
            FieldMeta::data("deleted_by")
                .label("Deleted By")
                .in_list_view(),
        )
        .field(FieldMeta::data("deleted_at").label("Deleted At"))
        .field(FieldMeta::small_text("reason").label("Reason"))
        .field(FieldMeta::data("restore_token").label("Restore Token"))
        .field(FieldMeta::data("restored_at").label("Restored At"))
        .field(
            FieldMeta::check("legal_hold")
                .label("Legal Hold")
                .default_value("0"),
        )
        .permission(system_manager())
}
