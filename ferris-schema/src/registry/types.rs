//! The metadata a registered DocType carries.

use crate::contract::meta::{CollectionView, DashboardView, DocView, NamingType};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DoctypeMeta {
    pub name: String,
    pub module: String,
    pub description: Option<String>,
    pub fields: Vec<FieldMeta>,
    pub permissions: Vec<PermissionRule>,
    pub naming_series: String,
    /// The declared naming type. `None` leaves it implicit: a non-empty
    /// `naming_series` means [`NamingType::Series`], no series means
    /// [`NamingType::Uuid`]. See [`DoctypeMeta::effective_naming_type`].
    pub naming_type: Option<NamingType>,
    /// The field the name is read from when `naming_type` is
    /// [`NamingType::Field`].
    pub naming_field: Option<String>,
    pub workflow: Option<String>,
    pub is_submittable: bool,
    pub is_tree: bool,
    pub is_virtual: bool,
    pub is_single: bool,
    pub is_child_table: bool,
    pub track_changes: bool,
    pub track_views: bool,
    pub allow_rename: bool,
    pub allow_copy: bool,
    pub allow_delete: bool,
    pub allow_print: bool,
    pub allow_email: bool,
    pub allow_attach: bool,
    pub allow_import: bool,
    pub allow_export: bool,
    pub allow_share: bool,
    pub allow_submit: bool,
    pub allow_cancel: bool,
    pub allow_amend: bool,
    pub indexes: Vec<IndexMeta>,
    pub search_fields: Vec<String>,
    pub sort_field: Option<String>,
    pub sort_order: Option<String>,
    pub image_field: Option<String>,
    pub title_field: Option<String>,
    pub subject_field: Option<String>,
    pub sender_field: Option<String>,
    pub recipient_field: Option<String>,
    pub content_field: Option<String>,
    pub date_field: Option<String>,
    pub color_field: Option<String>,
    pub icon_field: Option<String>,
    pub custom: bool,
    pub beta: bool,
    pub hidden: bool,
    pub read_only: bool,
    pub quick_entry: bool,
    /// Which view the Desk opens for this doctype's documents. Defaults to the
    /// built-in form; the engine serves the document whatever this says.
    ///
    /// `#[serde(default)]` on the struct makes both fields additive: a payload
    /// written before views were declarable omits them and gets the form.
    pub view: DocView,
    /// Backing of [`DocView::Dashboard`]: which dashboard a document opens.
    pub dashboard: DashboardView,
    /// Which collection view opens at `/desk/:module/:doctype`. `None` (the
    /// default) means the list, the route index every doctype has.
    ///
    /// The declaration is a redirect for the index, not a second route: the
    /// view still has its own URL (`…/kanban`, …), so a bookmark to the index
    /// keeps resolving and picking "Lista" keeps working.
    pub default_view: Option<CollectionView>,
    /// Kanban grouping field. Must be a multi-value `Select` or a `Link`;
    /// `None` keeps the historical derivation (the first multi-value `Select`,
    /// else `docstatus`).
    pub kanban_group_field: Option<String>,
    /// Fields shown on each kanban card, in order. Empty keeps the default
    /// card (title + creation date).
    pub kanban_card_fields: Vec<String>,
    /// Calendar end-date field: a document with both dates spans the days
    /// between them. `date_field` remains the start (and the only required
    /// one).
    pub calendar_end_field: Option<String>,
    /// Calendar title field, when the card should not use `title_field`.
    pub calendar_title_field: Option<String>,
    /// Tree parent field: a `Link` to this same doctype. Without it the tree
    /// view reports that the doctype is not configured for it.
    pub tree_parent_field: Option<String>,
    /// Tree label field, when rows should not use `title_field`.
    pub tree_label_field: Option<String>,
    /// Levels expanded when the tree is first opened (0 collapses every root
    /// that has children, 1 expands the roots, …). `None` means 1.
    pub tree_expanded_levels: Option<u8>,
    /// Gantt start-date field (required for a usable board).
    pub gantt_start_field: Option<String>,
    /// Gantt end-date field (required for a usable board).
    pub gantt_end_field: Option<String>,
    /// Gantt dependency field: a `Link` to this same doctype; the bar it names
    /// is drawn as the predecessor.
    pub gantt_dependency_field: Option<String>,
    /// Gantt progress field: `Percent`/`Int`/`Float`, 0–100.
    pub gantt_progress_field: Option<String>,
    pub in_create: bool,
    pub in_list_view: bool,
    pub in_dialog: bool,
    pub in_preview: bool,
    pub in_global_search: bool,
    pub in_sidebar: bool,
    pub in_dashboard: bool,
    pub in_calendar: bool,
    pub in_kanban: bool,
    pub in_gantt: bool,
    pub in_tree_view: bool,
    pub in_image_view: bool,
    pub in_file_view: bool,
    pub in_report_view: bool,
    pub in_print_view: bool,
    pub in_email_view: bool,
    pub in_communication_view: bool,
    pub in_assignment_view: bool,
    pub in_timeline_view: bool,
    pub in_activity_log: bool,
    pub in_version_log: bool,
    pub in_comment_log: bool,
    pub in_share_log: bool,
    pub in_permission_log: bool,
    pub in_workflow_log: bool,
    pub in_audit_log: bool,
    pub in_error_log: bool,
    pub in_scheduler_log: bool,
    pub in_background_job_log: bool,
    pub in_webhook_log: bool,
    pub in_integration_log: bool,
    pub in_oauth_log: bool,
    pub in_ldap_log: bool,
    pub in_saml_log: bool,
    pub in_mfa_log: bool,
    pub in_session_log: bool,
    pub in_login_log: bool,
    pub in_access_log: bool,
    pub in_api_log: bool,
    pub in_file_access_log: bool,
    pub in_print_log: bool,
    pub in_email_log: bool,
    pub in_sms_log: bool,
    pub in_notification_log: bool,
    pub in_like_log: bool,
    pub in_follow_log: bool,
    pub in_tag_log: bool,
    pub in_view_log: bool,
    pub in_search_log: bool,
    pub in_report_log: bool,
    pub in_dashboard_log: bool,
    pub in_workspace_log: bool,
    pub in_desk_log: bool,
    pub in_mobile_log: bool,
    pub in_offline_log: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FieldMeta {
    pub fieldname: String,
    pub label: Option<String>,
    pub fieldtype: String,
    pub options: Option<String>,
    pub default: Option<String>,
    pub reqd: bool,
    pub read_only: bool,
    pub hidden: bool,
    pub depends_on: Option<String>,
    pub description: Option<String>,
    pub in_list_view: bool,
    pub in_standard_filter: bool,
    pub in_global_search: bool,
    pub in_preview: bool,
    pub bold: bool,
    pub print_hide: bool,
    pub report_hide: bool,
    pub permlevel: u8,
    pub unique: bool,
    pub search_index: bool,
    pub sort_options: bool,
    pub allow_on_submit: bool,
    pub no_copy: bool,
    pub ignore_user_permissions: bool,
    pub ignore_xss_filter: bool,
    pub convert_to_user_timezone: bool,
    pub columns: u8,
    pub fetch_from: Option<String>,
    pub fetch_if_empty: bool,
    pub print_width: Option<String>,
    pub width: Option<String>,
    pub placeholder: Option<String>,
    pub max_length: Option<u32>,
    pub max_value: Option<String>,
    pub min_value: Option<String>,
    pub precision: Option<u8>,
    pub length: Option<u32>,
    pub read_only_on_load: bool,
    pub non_negative: bool,
    pub collapsible: bool,
    pub collapsible_depends_on: Option<String>,
    pub mandatory_depends_on: Option<String>,
    pub read_only_depends_on: Option<String>,
    pub hide_border: bool,
    pub in_filter: bool,
    pub remember_last_selected_value: bool,
    pub ignore_xss_filter_field: bool,
    /// Frappe's **Virtual** DocField
    /// (<https://docs.frappe.io/framework/user/en/basics/virtual_docfield>): the
    /// field is not persisted — it is computed on read and rendered read-only
    /// in the form. Defaulted so metadata emitted before it existed still
    /// deserializes.
    ///
    /// Written as a raw identifier because `virtual` is a reserved Rust keyword;
    /// the explicit `rename` keeps it under the plain name `virtual` on the wire.
    #[serde(rename = "virtual")]
    pub r#virtual: bool,
    /// Frappe's **Data Masking** flag
    /// (<https://docs.frappe.io/framework/data-masking>): the value stays in the
    /// document but is replaced by a mask on read unless the caller holds the
    /// `mask` permission at the field's `permlevel`.
    pub mask: bool,
}

// Re-export the core rule type so `DoctypeMeta.permissions` and the
// `#[doctype]`-generated `Document::permissions()` use a single type.
pub use ferris_core::PermissionRule;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct IndexMeta {
    pub name: String,
    pub fields: Vec<String>,
    pub unique: bool,
    pub index_type: i32,
}

impl DoctypeMeta {
    /// The single-field indexes over document fields that have no column: the
    /// fields the engine can answer exact-match filters on in SQL, through the
    /// JSON expression index [`crate::migration::classify_index`] materializes
    /// for the same declaration.
    ///
    /// This is the **pushdown** counterpart of DDL: the lazy engine and the
    /// migration runner create these indexes from `meta.indexes`, and `list`
    /// pushes `=` / `in` filters over the very same fields. Keeping the two
    /// derived from one classification means a filter can never be pushed for
    /// a field whose index did not get created, or vice versa.
    pub fn json_indexed_fields(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for index in &self.indexes {
            if let crate::migration::DeclaredIndex::JsonField(field) =
                crate::migration::classify_index(index)
            {
                if seen.insert(field.to_string()) {
                    out.push(field.to_string());
                }
            }
        }
        out
    }

    /// Canonical name of a Single `DocType`'s one document.
    ///
    /// A Single has exactly one document and, like Frappe, that document is
    /// named after the `DocType` itself (Frappe's `get_single` reads the doctype
    /// name as the primary key). The name is therefore derived, never generated
    /// from a naming series. Reading a non-single here is a caller bug; the
    /// engine refuses to use this name for a non-single before it gets this far.
    pub fn single_name(&self) -> &str {
        &self.name
    }

    /// The naming type the engine applies to this doctype's new documents.
    ///
    /// An undeclared doctype keeps the pre-existing behaviour (series if it has
    /// one, UUID otherwise), so adding the declaration is fully backward
    /// compatible; an explicit `naming_type` always wins.
    #[must_use]
    pub fn effective_naming_type(&self) -> NamingType {
        NamingType::resolve(self.naming_type, &self.naming_series)
    }
}
