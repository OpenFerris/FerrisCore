//! The `Access Log` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta, RolePermission};

use super::common::MODULE;

/// Read-only like `Activity Log`: the record of file access is evidence, so
/// `System Manager` can read it but no role is granted write or delete
/// (SEC-09).
fn audit_read() -> RolePermission {
    RolePermission::new("System Manager").read()
}

/// The audit trail of data exports and file access.
#[must_use]
pub fn access_log_meta() -> DoctypeMeta {
    DoctypeMeta::new("Access Log", MODULE)
        .description("The audit trail of data exports and file access.")
        .sort("creation", "desc")
        .read_only()
        .field(FieldMeta::link("user", "User").label("User").in_list_view())
        .field(FieldMeta::data("export_from").label("Export From"))
        .field(FieldMeta::data("reference_document").label("Reference Document"))
        .field(
            FieldMeta::data("file_type")
                .label("File Type")
                .in_list_view(),
        )
        .permission(audit_read())
}
