//! The registration order of every DocType this crate declares.
//!
//! One list per section, and one entry point that walks them: the host calls
//! [`register_doctypes`] on every boot, so the order here is the order the
//! schema registry sees, and it is the only place that order is decided.

use ferris_schema::{DoctypeMeta, register_doctype};

use super::dashboards::{dashboard_layout_meta, dashboard_settings_meta};
use super::data::{
    document_naming_rule_condition_meta, document_naming_rule_meta, document_naming_settings_meta,
};
use super::desk::{notification_log_meta, tag_meta, todo_meta};
use super::system::{
    desired_state_meta, desk_layout_meta, event_meta, notification_meta, notification_settings_meta,
};

/// The framework DocTypes every install has, in registration order.
///
/// All six are `hidden`: they are reached through the desk panels, never as
/// workspace links.
#[must_use]
pub fn framework_doctype_metas() -> Vec<DoctypeMeta> {
    vec![
        notification_meta(),
        event_meta(),
        notification_settings_meta(),
        desk_layout_meta(),
        desired_state_meta(),
        dashboard_layout_meta(),
        dashboard_settings_meta(),
    ]
}

/// The Data section DocTypes, in registration order.
#[must_use]
pub fn data_doctype_metas() -> Vec<DoctypeMeta> {
    vec![
        // Data
        document_naming_rule_meta(),
        document_naming_settings_meta(),
        // Desk (the feed helpers)
        todo_meta(),
        tag_meta(),
        notification_log_meta(),
    ]
}

/// Every DocType this crate declares, in registration order.
#[must_use]
pub fn doctype_metas() -> Vec<DoctypeMeta> {
    let mut metas = framework_doctype_metas();
    metas.extend(data_doctype_metas());
    metas
}

/// Register `ferris-api`'s DocTypes in the global schema registry.
///
/// Idempotent: re-registering overwrites the same entries, so it is safe to
/// call on every boot and after a hot reload. Tables themselves are created
/// lazily by the document engine the first time a DocType is touched.
pub fn register_doctypes() {
    for meta in doctype_metas() {
        register_doctype(meta);
    }
    // A child table is not a workspace item, so it is registered here instead of
    // from `data_doctype_metas()` (which asserts every entry is visible).
    register_doctype(document_naming_rule_condition_meta());
    // `Tag Link` is declared in [`crate::tags`] (it is an implementation detail
    // of the standard `tags` field, not a Desk document), so it is registered
    // from here rather than listed in `doctype_metas`.
    crate::tags::register();
}
