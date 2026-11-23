//! The `/api/doctype/meta/{doctype}` payload the Desk renders.

use serde::{Deserialize, Serialize};

use super::field::{DynField, DynIndex};
use super::naming::NamingType;
use super::permissions::{DynCustomPermission, DynPermissionRule};
use super::relations::{DynDocAction, DynDocLink};
use super::view::{CollectionView, DashboardView, DocView};

/// Metadata of one DocType, as served by `GET /api/doctype/meta/{doctype}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DynMeta {
    pub name: String,
    #[serde(default)]
    pub module: String,
    /// The DocType's human label, already resolved for the request's locale by
    /// the host. `None` only in payloads written before the field existed.
    #[serde(default)]
    pub label: Option<String>,
    pub title_field: Option<String>,
    pub search_fields: Vec<String>,
    pub date_field: Option<String>,
    /// The doctype's declared naming series (`TASK-.#####`), empty when it
    /// declares none. The Desk autocompletes the id field with it in `new`.
    #[serde(default)]
    pub naming_series: String,
    /// The declared naming type. `None` means "implicit": resolve it with
    /// [`NamingType::resolve`] against `naming_series`.
    #[serde(default)]
    pub naming_type: Option<NamingType>,
    /// The field the name is read from when the type is [`NamingType::Field`].
    #[serde(default)]
    pub naming_field: Option<String>,
    /// Default ordering of the doctype's lists. Additive and defaulted: the
    /// engine could apply them as customizations before the Desk could read
    /// them, which made them editable nowhere.
    #[serde(default)]
    pub sort_field: Option<String>,
    #[serde(default)]
    pub sort_order: Option<String>,
    /// The doctype's human description. Also a DocType-level customization.
    #[serde(default)]
    pub description: Option<String>,
    pub fields: Vec<DynField>,
    pub is_submittable: bool,
    pub allow_submit: bool,
    pub allow_cancel: bool,
    pub read_only: bool,
    pub is_virtual: bool,
    pub hidden: bool,
    /// properties). Child documents have `parent`, `parenttype`,
    /// `parentfield` and `idx` attached by the engine (Frappe child table
    /// semantics).
    #[serde(default)]
    pub is_child_table: bool,
    pub is_single: bool,
    /// Which view the Desk opens for this doctype's documents. The default is
    /// the built-in form; the engine serves the document either way.
    #[serde(default)]
    pub view: DocView,
    /// Backing of [`DocView::Dashboard`]: which dashboard the document opens.
    #[serde(default)]
    pub dashboard: DashboardView,
    /// Which collection view opens at `/desk/:module/:doctype`. `None` means
    /// the list.
    #[serde(default)]
    pub default_view: Option<CollectionView>,
    /// Kanban grouping field (`Select`/`Link`); `None` keeps the derivation.
    #[serde(default)]
    pub kanban_group_field: Option<String>,
    /// Fields shown on each kanban card, in order (empty = title + creation).
    #[serde(default)]
    pub kanban_card_fields: Vec<String>,
    /// Calendar end-date field; `date_field` is the start.
    #[serde(default)]
    pub calendar_end_field: Option<String>,
    /// Calendar title field override.
    #[serde(default)]
    pub calendar_title_field: Option<String>,
    /// Tree parent field (`Link` to the same doctype).
    #[serde(default)]
    pub tree_parent_field: Option<String>,
    /// Tree label field override.
    #[serde(default)]
    pub tree_label_field: Option<String>,
    /// Levels expanded when the tree first opens (`None` = 1).
    #[serde(default)]
    pub tree_expanded_levels: Option<u8>,
    /// Gantt start-date field.
    #[serde(default)]
    pub gantt_start_field: Option<String>,
    /// Gantt end-date field.
    #[serde(default)]
    pub gantt_end_field: Option<String>,
    /// Gantt dependency field (`Link` to the same doctype).
    #[serde(default)]
    pub gantt_dependency_field: Option<String>,
    /// Gantt progress field (`Percent`/`Int`/`Float`, 0–100).
    #[serde(default)]
    pub gantt_progress_field: Option<String>,
    /// UI affordance flags (authorization still flows through the engine).
    #[serde(default)]
    pub allow_amend: bool,
    #[serde(default)]
    pub allow_delete: bool,
    #[serde(default)]
    pub allow_copy: bool,
    #[serde(default)]
    pub allow_print: bool,
    #[serde(default)]
    pub allow_rename: bool,
    /// `is_tree` is what makes the tree view legal for a doctype; the view flag
    /// alone is not enough, because the tree needs the parent field the
    /// metadata declares.
    #[serde(default)]
    pub is_tree: bool,
    #[serde(default)]
    pub track_changes: bool,
    #[serde(default)]
    pub quick_entry: bool,
    /// The workflow the doctype is bound to, when it declares one. The engine
    /// enforces it; the Desk only uses it to explain why a transition was
    /// refused.
    #[serde(default)]
    pub workflow: Option<String>,
    /// Field roles the framework resolves generically (image, subject, sender,
    /// recipient, content, colour, icon) instead of hardcoding a field name in
    /// a view.
    #[serde(default)]
    pub image_field: Option<String>,
    #[serde(default)]
    pub subject_field: Option<String>,
    #[serde(default)]
    pub sender_field: Option<String>,
    #[serde(default)]
    pub recipient_field: Option<String>,
    #[serde(default)]
    pub content_field: Option<String>,
    #[serde(default)]
    pub color_field: Option<String>,
    #[serde(default)]
    pub icon_field: Option<String>,
    /// The `allow-*` surface, mirroring the flags the engine enforces. They are
    /// defaulted so a payload emitted before the field existed still parses.
    #[serde(default)]
    pub allow_email: bool,
    #[serde(default)]
    pub allow_attach: bool,
    #[serde(default)]
    pub allow_import: bool,
    #[serde(default)]
    pub allow_export: bool,
    #[serde(default)]
    pub allow_share: bool,
    /// Where the doctype appears: create dialog, quick entry, preview, global
    /// search, sidebar and the dashboard catalogue.
    #[serde(default)]
    pub in_create: bool,
    #[serde(default)]
    pub in_dialog: bool,
    #[serde(default)]
    pub in_preview: bool,
    #[serde(default)]
    pub in_global_search: bool,
    #[serde(default)]
    pub in_sidebar: bool,
    #[serde(default)]
    pub in_dashboard: bool,
    /// The remaining view flags a stored doctype can set.
    #[serde(default)]
    pub in_image_view: bool,
    #[serde(default)]
    pub in_file_view: bool,
    #[serde(default)]
    pub in_print_view: bool,
    #[serde(default)]
    pub in_email_view: bool,
    /// View capabilities from the doctype's definition. The top-bar mode
    /// selector only offers the views the definition actually enables.
    #[serde(default)]
    pub in_list_view: bool,
    #[serde(default)]
    pub in_calendar: bool,
    #[serde(default)]
    pub in_kanban: bool,
    #[serde(default)]
    pub in_gantt: bool,
    #[serde(default)]
    pub in_tree_view: bool,
    #[serde(default)]
    pub in_report_view: bool,
    /// Indexes the doctype declares. The Desk shows the fields they cover as
    /// list columns, so declaring an index is also a display decision.
    #[serde(default)]
    pub indexes: Vec<DynIndex>,
    /// Per-role permission rules so the Desk can decide client-side which
    /// affordances (create, …) to *offer*; real authorization still flows
    /// through the engine.
    #[serde(default)]
    pub permissions: Vec<DynPermissionRule>,
    /// Document actions the server reads from the doctype's stored `DocType
    /// Action` rows (forms, list views and reports). Older payloads carry no
    /// key and default to empty, so the Desk simply offers no custom actions.
    #[serde(default)]
    pub actions: Vec<DynDocAction>,
    /// Document links the server reads from the stored `DocType Link` rows
    /// (the Connections panel). Older payloads default to empty.
    #[serde(default)]
    pub links: Vec<DynDocLink>,
    /// Custom permission types declared for this doctype, read from the stored
    /// `Permission Type` rows. They name rights beyond the built-in flags, and
    /// older payloads default to empty.
    #[serde(default)]
    pub custom_permissions: Vec<DynCustomPermission>,
}

impl DynMeta {
    /// The naming type this doctype resolves to, as the engine will apply it.
    ///
    /// Available without the `native` feature, exactly like the rest of the
    /// wire contract, so the Desk can decide whether the id field is editable.
    #[must_use]
    pub fn naming(&self) -> NamingType {
        NamingType::resolve(self.naming_type, &self.naming_series)
    }

    /// Whether a caller holding `roles` should be **offered** a "create"
    /// affordance for this doctype.
    ///
    /// This is an affordance rule, not authorization: the engine still decides
    /// on the request, and a wrong `true` only produces a 403. It lives here,
    /// next to the `permissions` payload it reads, so the Desk and any other
    /// contract consumer share one implementation instead of each re-deriving
    /// the `System Manager`/`Administrator` passthrough (WS9b).
    ///
    /// A Single is excluded: its one document is materialised by the framework,
    /// not created by a user.
    #[must_use]
    pub fn offers_create(&self, roles: &[String]) -> bool {
        if self.read_only || self.is_virtual || self.is_single {
            return false;
        }
        if roles
            .iter()
            .any(|role| role == "System Manager" || role == "Administrator")
        {
            return true;
        }
        self.permissions
            .iter()
            .any(|rule| rule.create && roles.contains(&rule.role))
    }
}

#[cfg(feature = "native")]
impl DynMeta {
    /// Convert registry metadata into the portable mirror through its wire
    /// representation — the exact bytes the REST endpoint emits.
    ///
    /// Going via JSON rather than a field-by-field copy is what guarantees the
    /// SSR path and the hydrate path agree: if the mirror drifts from the
    /// endpoint's shape, this is where it fails, in SSR, instead of in a
    /// browser.
    pub fn from_serde_meta(m: &crate::registry::DoctypeMeta) -> Result<Self, String> {
        let value = serde_json::to_value(m).map_err(|e| e.to_string())?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
}
