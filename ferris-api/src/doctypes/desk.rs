//! The Desk-feed helper DocTypes: personal tasks, tags, bulk edits and the
//! notification log.
//!
//! Visible Desk items like the Data family, but they back desk widgets rather
//! than the import/export tooling.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::constants::{DESK_MODULE, TAG};
use super::support::system_manager;

/// A personal task on the Desk.
///
/// The engine-facing field is `title`: the search/report/GraphQL paths in this
/// workspace all address a ToDo by it, so the declaration matches them.
#[must_use]
pub fn todo_meta() -> DoctypeMeta {
    DoctypeMeta::new("ToDo", DESK_MODULE)
        .description("A personal task on the Desk.")
        .title_field("title")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::data("title")
                .label("Title")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::small_text("description").label("Description"))
        // Free-form `Data`, not a `Select`: the search/report/GraphQL fixtures
        // in this workspace use lowercase tokens ("open"/"closed") that a
        // Frappe-style capitalised enum would reject. The declaration grows an
        // enum when something actually reads one.
        .field(
            FieldMeta::data("status")
                .label("Status")
                .default_value("open")
                .in_list_view(),
        )
        .field(
            FieldMeta::select("priority", "Low\nMedium\nHigh")
                .label("Priority")
                .default_value("Medium")
                .in_list_view(),
        )
        .field(FieldMeta::date("date").label("Due Date"))
        .field(FieldMeta::link("allocated_to", "User").label("Allocated To"))
        .field(FieldMeta::link("reference_type", "DocType").label("Reference DocType"))
        .field(FieldMeta::data("reference_name").label("Reference Document"))
        .permission(system_manager())
}

/// A free-form label attached to documents.
///
/// This is the catalogue behind the standard `tags` field every `DocType`
/// carries (`ferris_schema::tags_field`): the field holds the label text, and
/// this `DocType` is where those labels are listed. It is a normal, visible
/// Desk item so operators can curate the set (rename, delete a tag a document
/// still references).
///
/// A row is created on demand by [`crate::tags::sync_document`] whenever a
/// document saves a label it does not yet know; the per-document associations
/// live in [`crate::tags::TAG_LINK`], not here.
#[must_use]
pub fn tag_meta() -> DoctypeMeta {
    DoctypeMeta::new(TAG, DESK_MODULE)
        .description("A free-form label that can be attached to documents.")
        .title_field("tag")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::data("tag")
                .label("Tag")
                .required()
                .unique()
                .in_list_view(),
        )
        .permission(system_manager())
}

/// A one-shot bulk edit across documents of a DocType.
#[must_use]
pub fn bulk_update_meta() -> DoctypeMeta {
    DoctypeMeta::new("Bulk Update", DESK_MODULE)
        .description("A one-shot bulk edit across documents of a DocType.")
        .title_field("document_type")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::link("document_type", "DocType")
                .label("Document Type")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::data("field")
                .label("Field")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("value").label("Value"))
        .field(FieldMeta::code("condition").label("Condition"))
        .field(
            FieldMeta::select("status", "Pending\nCompleted\nFailed")
                .label("Status")
                .default_value("Pending")
                .in_list_view(),
        )
        .permission(system_manager())
}

/// One notification as it was delivered to a user.
#[must_use]
pub fn notification_log_meta() -> DoctypeMeta {
    DoctypeMeta::new("Notification Log", DESK_MODULE)
        .description("A notification as it was delivered to one user.")
        .title_field("subject")
        .sort("creation", "desc")
        .allow_delete()
        .field(
            FieldMeta::data("subject")
                .label("Subject")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("for_user", "User")
                .label("For User")
                .in_list_view(),
        )
        .field(
            FieldMeta::select("type", "Mention\nAssignment\nShare\nAlert\nSystem")
                .label("Type")
                .default_value("Alert")
                .in_list_view(),
        )
        .field(FieldMeta::link("document_type", "DocType").label("Document Type"))
        .field(FieldMeta::data("document_name").label("Document Name"))
        .field(FieldMeta::check("read").label("Read").default_value("0"))
        .permission(system_manager())
}
