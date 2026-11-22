//! DocTypes declared by `ferris-schema` (the Build and GEO sections).
//!
//! The schema crate owns the registry ([`crate::registry`]) and the wire
//! contract ([`crate::contract`]); it declares the framework metadata DocTypes
//! here — the customization/scripting/view designers, the module registry and
//! global-search settings (Desk section **Build**) and the geography tables
//! (section **GEO**) — in line with ``
//!
//! Three of them are **Singles** (`is_single`): `Customize Form` (the form
//! designer's working row), `Navbar Settings` and `Global Search Settings`. A
//! Single is one fixed row named after its DocType, read and written through
//! `get_single`/`save_single` (§13); it has no list view and its row is created
//! lazily with the field defaults declared here.
//!
//! Field sets are deliberately Frappe-shaped but trimmed to what a declaration
//! needs; a DocType grows fields when something reads them, not before.
//!
//! The declarations are grouped by area — [`customization`], [`scripting`],
//! [`views`], [`form_meta`], [`workspace`], [`modules`] and [`geo`] — while
//! [`all`] keeps the one registration order every consumer iterates.
#![allow(clippy::doc_markdown)]

mod all;
mod consts;
mod customization;
mod form_meta;
mod geo;
mod modules;
mod permissions;
mod scripting;
mod system;
mod views;
mod workspace;

pub use all::{doctype_metas, register_doctypes};
pub use consts::{BUILD_MODULE, GEO_MODULE};
pub use customization::{
    custom_doc_perm_meta, custom_field_meta, custom_translation_meta, customize_form_meta,
    navbar_settings_meta, property_setter_meta,
};
pub use form_meta::{
    doctype_action_meta, doctype_link_meta, form_tour_meta, form_tour_step_meta,
    permission_type_meta,
};
pub use geo::{country_meta, currency_meta, language_meta, translation_meta};
pub use modules::{global_search_settings_meta, module_def_meta};
pub use scripting::{client_script_meta, server_script_meta};
pub use system::deleted_document_meta;
pub use views::{
    calendar_view_meta, dashboard_meta, dashboard_widget_meta, kanban_board_meta, list_filter_meta,
    page_meta, report_meta, widget_meta, widget_param_meta,
};
pub use workspace::{
    workspace_chart_meta, workspace_custom_block_meta, workspace_link_meta, workspace_meta,
    workspace_number_card_meta, workspace_quick_list_meta, workspace_shortcut_meta,
};

#[cfg(test)]
mod tests;
