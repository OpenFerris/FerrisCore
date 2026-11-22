//! Scripting DocTypes: browser and server scripts (Desk section **Build**).

use crate::registry::{DoctypeMeta, FieldMeta};

use super::consts::BUILD_MODULE;

use super::permissions::system_manager;

/// A script the Desk runs in the browser.
#[must_use]
pub fn client_script_meta() -> DoctypeMeta {
    DoctypeMeta::new("Client Script", BUILD_MODULE)
        .description("A script the Desk runs in the browser.")
        .title_field("dt")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::link("dt", "DocType")
                .label("Document Type")
                .in_list_view(),
        )
        .field(
            FieldMeta::check("enabled")
                .label("Enabled")
                .default_value("1")
                .in_list_view(),
        )
        .field(FieldMeta::code("script").label("Script").options("JS"))
        .permission(system_manager())
}

/// A script the server runs on a document event or API call.
#[must_use]
pub fn server_script_meta() -> DoctypeMeta {
    DoctypeMeta::new("Server Script", BUILD_MODULE)
        .description("A script the server runs on a document event or API call.")
        .title_field("name")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::data("name").label("Name").in_list_view())
        .field(
            FieldMeta::select("script_type", "DocType Event\nAPI")
                .label("Script Type")
                .default_value("DocType Event")
                .in_list_view(),
        )
        .field(
            FieldMeta::select(
                "event_type",
                "Before Insert\nAfter Insert\nBefore Save\nAfter Save\nBefore Submit\n\
                 After Submit\nBefore Cancel\nAfter Cancel\nBefore Delete\nAfter Delete",
            )
            .label("Event Type"),
        )
        .field(FieldMeta::link("reference_doctype", "DocType").label("Reference DocType"))
        .field(
            FieldMeta::check("enabled")
                .label("Enabled")
                .default_value("1"),
        )
        .field(FieldMeta::code("script").label("Script").options("Python"))
        .permission(system_manager())
}
