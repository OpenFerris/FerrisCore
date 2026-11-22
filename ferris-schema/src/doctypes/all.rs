//! The ordered registration list every consumer iterates.

use crate::registry::DoctypeMeta;

use super::{
    calendar_view_meta, client_script_meta, country_meta, currency_meta, custom_doc_perm_meta,
    custom_field_meta, custom_translation_meta, customize_form_meta, dashboard_meta,
    dashboard_widget_meta, deleted_document_meta, doctype_action_meta, doctype_link_meta,
    form_tour_meta, form_tour_step_meta, global_search_settings_meta, kanban_board_meta,
    language_meta, list_filter_meta, module_def_meta, navbar_settings_meta, page_meta,
    permission_type_meta, property_setter_meta, report_meta, server_script_meta, translation_meta,
    widget_meta, widget_param_meta, workspace_chart_meta, workspace_custom_block_meta,
    workspace_link_meta, workspace_meta, workspace_number_card_meta, workspace_quick_list_meta,
    workspace_shortcut_meta,
};

/// Every DocType this crate declares, in registration order.
#[must_use]
pub fn doctype_metas() -> Vec<DoctypeMeta> {
    vec![
        // Build — Customization
        customize_form_meta(),
        custom_field_meta(),
        property_setter_meta(),
        custom_doc_perm_meta(),
        custom_translation_meta(),
        navbar_settings_meta(),
        // Build — Scripting
        client_script_meta(),
        server_script_meta(),
        // Build — Views
        report_meta(),
        dashboard_meta(),
        dashboard_widget_meta(),
        widget_meta(),
        widget_param_meta(),
        kanban_board_meta(),
        calendar_view_meta(),
        list_filter_meta(),
        page_meta(),
        // Build — Form metadata (actions, links, permission types)
        doctype_action_meta(),
        doctype_link_meta(),
        permission_type_meta(),
        form_tour_meta(),
        form_tour_step_meta(),
        // Build — Workspace (the Workspace DocType and its child tables)
        workspace_meta(),
        workspace_link_meta(),
        workspace_shortcut_meta(),
        workspace_chart_meta(),
        workspace_number_card_meta(),
        workspace_quick_list_meta(),
        workspace_custom_block_meta(),
        // Build — Modules
        module_def_meta(),
        // Build — Settings
        global_search_settings_meta(),
        // Build — Trash (soft-deleted documents, audit and restore)
        deleted_document_meta(),
        // GEO
        country_meta(),
        currency_meta(),
        language_meta(),
        translation_meta(),
    ]
}

/// Register this crate's DocTypes in the global schema registry.
///
/// Idempotent: re-registering overwrites the same keys, so it is safe on every
/// boot and after a hot reload. Tables themselves are created lazily by the
/// document engine the first time a DocType is touched.
pub fn register_doctypes() {
    for meta in doctype_metas() {
        crate::registry::register_doctype(meta);
    }
}
