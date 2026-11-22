//! The `Permission Inspector` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// A tool that shows why a user may or may not reach a document.
#[must_use]
pub fn permission_inspector_meta() -> DoctypeMeta {
    DoctypeMeta::new("Permission Inspector", MODULE)
        .description("Shows why a user may or may not reach a document.")
        .sort("modified", "desc")
        .field(FieldMeta::link("user", "User").label("User").in_list_view())
        .field(
            FieldMeta::link("doc_type", "DocType")
                .label("Document Type")
                .in_list_view(),
        )
        .permission(system_manager())
}
