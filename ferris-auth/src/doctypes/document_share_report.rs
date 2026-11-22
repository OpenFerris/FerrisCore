//! The `Document Share Report` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// Report: who a set of documents is shared with.
#[must_use]
pub fn document_share_report_meta() -> DoctypeMeta {
    DoctypeMeta::new("Document Share Report", MODULE)
        .description("Report: who a set of documents is shared with.")
        .sort("modified", "desc")
        .field(FieldMeta::link("user", "User").label("User").in_list_view())
        .field(FieldMeta::link("reference_doctype", "DocType").label("Reference DocType"))
        .field(
            FieldMeta::data("reference_docname")
                .label("Reference Document")
                .in_list_view(),
        )
        .permission(system_manager())
}
