//! The `User Permission` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// A per-user grant narrowing access to one value of a linked `DocType`.
///
/// The shape is Frappe's: `allow` is the **`DocType`** whose values are allowed
/// and `for_value` is the one allowed value of it. It used to be declared as a
/// `Select` of literal `DocType`/`User`/`Role` labels, which named the *kind* of
/// value rather than the `DocType` to restrict, so the row could never describe
/// a restriction the engine could read. `ferris-auth::user_permission` stores
/// and loads these rows; `ferris-document` enforces them on reads and writes.
#[must_use]
pub fn user_permission_meta() -> DoctypeMeta {
    DoctypeMeta::new("User Permission", MODULE)
        .description("A per-user grant narrowing access to one value of a linked DocType.")
        .title_field("for_value")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("user", "User")
                .label("User")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("allow", "DocType")
                .label("Allow")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::data("for_value")
                .label("For Value")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("applicable_for", "DocType")
                .label("Applicable For")
                .description("Restrict only this DocType; empty applies to Allow itself."),
        )
        .field(
            FieldMeta::check("apply_to_all_doctypes")
                .label("Apply to all DocTypes")
                .default_value("0")
                .description("Restrict every DocType that links to Allow."),
        )
        .field(
            FieldMeta::check("hide_descendants")
                .label("Hide Descendants")
                .default_value("0")
                .description("Include the descendants of a tree Allow value."),
        )
        .permission(system_manager())
}
