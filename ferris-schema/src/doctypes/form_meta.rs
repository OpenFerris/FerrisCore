//! Per-doctype form metadata: actions, links and permission types.

use crate::registry::{DoctypeMeta, FieldMeta};

use super::consts::BUILD_MODULE;

use super::permissions::{all_users_read, system_manager};

/// A document action declared on a DocType (Frappe's `DocType Action`).
///
/// Actions are the buttons a form, list or report offers: `Server Action`
/// calls a method, `Route` opens a Desk route. They are stored per doctype, so
/// the metadata feed carries them and the Desk never hardcodes one. Frappe's
/// controller lifecycle that runs them is described at
/// <https://docs.frappe.io/framework/user/en/basics/doctypes/controllers>.
#[must_use]
pub fn doctype_action_meta() -> DoctypeMeta {
    DoctypeMeta::new("DocType Action", BUILD_MODULE)
        .description("A document action declared on a DocType.")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("document_type", "DocType")
                .label("Document Type")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::select("action_type", "Server Action\nRoute")
                .label("Action Type")
                .default_value("Server Action"),
        )
        .field(FieldMeta::data("action").label("Action").required())
        .field(FieldMeta::data("label").label("Label").in_list_view())
        .field(FieldMeta::data("group").label("Group"))
        .field(FieldMeta::check("hidden").label("Hidden"))
        .field(FieldMeta::check("custom").label("Custom"))
        .permission(system_manager())
}

/// A document link declared on a DocType (Frappe's `DocType Link`).
///
/// A link feeds the Connections panel: it names the linked doctype
/// (`link_doctype`) and the field on this doctype that points at it
/// (`link_fieldname`), optionally inside a child table (`table_fieldname`).
/// Declared alongside the controller surface at
/// <https://docs.frappe.io/framework/user/en/basics/doctypes/controllers>.
#[must_use]
pub fn doctype_link_meta() -> DoctypeMeta {
    DoctypeMeta::new("DocType Link", BUILD_MODULE)
        .description("A document link declared on a DocType.")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("document_type", "DocType")
                .label("Document Type")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("link_doctype", "DocType")
                .label("Link DocType")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::data("link_fieldname")
                .label("Link Fieldname")
                .required(),
        )
        .field(FieldMeta::data("table_fieldname").label("Table Fieldname"))
        .field(FieldMeta::data("group").label("Group"))
        .field(FieldMeta::check("hidden").label("Hidden"))
        .field(FieldMeta::check("custom").label("Custom"))
        .permission(system_manager())
}

/// A custom permission type declared for a DocType (Frappe's `Permission Type`).
///
/// Frappe's permission types add app-defined rights (`approve`, `verify`, …)
/// beyond the fixed read/write/… flags; this row records one (`perm_type`) and
/// the role that grants it. Reference:
/// <https://docs.frappe.io/framework/permission-types>.
#[must_use]
pub fn permission_type_meta() -> DoctypeMeta {
    DoctypeMeta::new("Permission Type", BUILD_MODULE)
        .description("A custom permission type declared for a DocType.")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::data("perm_type")
                .label("Permission Type")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("document_type", "DocType")
                .label("Document Type")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("label").label("Label"))
        .field(FieldMeta::check("custom").label("Custom"))
        .permission(system_manager())
}

/// A guided tour of a DocType form (Frappe's `Form Tour`).
///
/// A tour is data, not code: the `Form Tour Step` child table names the field
/// to highlight, the copy to show and the condition that decides whether the
/// next step is reachable. The Desk reads the rows through
/// `GET /api/doctype/form-tours/{doctype}` and renders the overlay; a tour a
/// site does not have is simply absent.
///
/// Reference: <https://docs.frappe.io/framework/user/en/form-tours>.
#[must_use]
pub fn form_tour_meta() -> DoctypeMeta {
    DoctypeMeta::new("Form Tour", BUILD_MODULE)
        .description("A guided tour of a DocType form.")
        .title_field("title")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::data("title")
                .label("Title")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("reference_doctype", "DocType")
                .label("Reference DocType")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::check("is_standard").label("Is Standard"))
        .field(FieldMeta::check("save_on_completion").label("Save on Completion"))
        .field(FieldMeta::check("show_first_document_tour").label("Show First Document Tour"))
        .field(FieldMeta::check("include_name_field").label("Include Name Field"))
        .field(FieldMeta::table("steps", "Form Tour Step").label("Steps"))
        .permission(system_manager())
        .permission(all_users_read())
}

/// One step of a [`form_tour_meta`] tour.
///
/// `fieldname` is the field to highlight; `parent_field` names the child-table
/// field when the target lives inside a row (`is_table_field`). `position` is a
/// popover placement hint (`bottom`, `top`, `left`, `right`). `next_condition`
/// is the same expression language `depends_on` uses: the step is skipped when
/// it evaluates false.
#[must_use]
pub fn form_tour_step_meta() -> DoctypeMeta {
    DoctypeMeta::new("Form Tour Step", BUILD_MODULE)
        .description("One step of a Form Tour.")
        .child_table()
        .sort("idx", "asc")
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("parent", "Form Tour")
                .label("Form Tour")
                .required(),
        )
        .field(FieldMeta::data("fieldname").label("Fieldname").required())
        .field(FieldMeta::data("title").label("Title").required())
        .field(FieldMeta::text("description").label("Description"))
        .field(
            FieldMeta::select("position", "bottom\ntop\nleft\nright")
                .label("Position")
                .default_value("bottom"),
        )
        .field(FieldMeta::code("next_condition").label("Next Condition"))
        .field(FieldMeta::check("is_table_field").label("Is Table Field"))
        .field(FieldMeta::data("parent_field").label("Parent Field"))
        .permission(system_manager())
}
