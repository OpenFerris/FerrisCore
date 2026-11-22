//! The `Permitted Documents For User` report DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// Report: which documents one user may read.
#[must_use]
pub fn permitted_documents_for_user_meta() -> DoctypeMeta {
    DoctypeMeta::new("Permitted Documents For User", MODULE)
        .description("Report: which documents one user may read.")
        .sort("modified", "desc")
        .field(FieldMeta::link("user", "User").label("User").in_list_view())
        .field(
            FieldMeta::link("doc_type", "DocType")
                .label("Document Type")
                .in_list_view(),
        )
        .permission(system_manager())
}
