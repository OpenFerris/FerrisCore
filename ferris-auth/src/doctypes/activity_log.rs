//! The `Activity Log` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta, RolePermission};

use super::common::MODULE;

/// The audit trail is append-only: every role, `System Manager` included,
/// reads it and nobody writes or deletes it through the generic engine.
///
/// The engine still has to persist rows (login/logout), so the write goes
/// through the explicit internal producer
/// (`ferris_server::observability::record_activity`) and never through a REST
/// permission grant. The append-only guard installed with the system DocTypes
/// rejects `update`/`delete` even for a role the core permission engine would
/// otherwise short-circuit (SEC-07/08/09/AUTH-17).
fn audit_read() -> RolePermission {
    RolePermission::new("System Manager").read()
}

/// The audit trail of user operations.
#[must_use]
pub fn activity_log_meta() -> DoctypeMeta {
    DoctypeMeta::new("Activity Log", MODULE)
        .description("The audit trail of user operations.")
        .sort("creation", "desc")
        .read_only()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(FieldMeta::link("user", "User").label("User").in_list_view())
        .field(FieldMeta::data("subject").label("Subject").in_list_view())
        .field(
            FieldMeta::data("operation")
                .label("Operation")
                .in_list_view(),
        )
        .field(FieldMeta::data("status").label("Status").in_list_view())
        .field(FieldMeta::data("ip_address").label("IP Address"))
        .permission(audit_read())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_logs_are_read_only() {
        for meta in [
            activity_log_meta(),
            super::super::access_log::access_log_meta(),
        ] {
            assert!(!meta.allow_delete, "{} must not offer delete", meta.name);
            assert!(meta.read_only, "{} must be declared read-only", meta.name);
            assert!(
                meta.permissions
                    .iter()
                    .any(|rule| rule.role == "System Manager" && rule.read),
                "{} must stay readable by System Manager",
                meta.name
            );
            assert!(
                meta.permissions
                    .iter()
                    .all(|rule| !rule.write && !rule.create && !rule.delete),
                "{} must grant no write/create/delete",
                meta.name
            );
        }
    }
}
