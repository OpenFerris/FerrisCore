use crate::contract::meta::{CollectionView, DashboardView, DocView, NamingType};
use crate::{DoctypeMeta, FieldMeta, IndexMeta};
use ferris_core::PermissionRule;

impl DoctypeMeta {
    /// A new DocType with only its identity set.
    ///
    /// Everything else defaults to the registry zero: no flags, no fields, no
    /// permissions, empty `naming_series` (the engine generates opaque names).
    #[must_use]
    pub fn new(name: impl Into<String>, module: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            module: module.into(),
            ..Self::default()
        }
    }

    /// Replace the human description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the naming series (`"PRINT-####"`). Empty means opaque UUID names.
    #[must_use]
    pub fn naming_series(mut self, series: impl Into<String>) -> Self {
        self.naming_series = series.into();
        self
    }

    /// Declare how the name is minted (`Series`, `NamingSeries`, `Uuid`,
    /// `AutoIncrement`, `Field`, `Prompt`, `Expression`, `ExpressionOldStyle`,
    /// `Random`, `Script`).
    ///
    /// Without this, the type is implicit: a non-empty series means
    /// [`NamingType::Series`], no series means [`NamingType::Uuid`]. Where the
    /// series already says it, prefer this only to *change* the strategy.
    #[must_use]
    pub fn naming_type(mut self, naming_type: NamingType) -> Self {
        self.naming_type = Some(naming_type);
        self
    }

    /// Name new documents after the value of `field`, when `naming_type` is
    /// [`NamingType::Field`]. The engine rejects an insert whose field is empty.
    #[must_use]
    pub fn naming_field(mut self, field: impl Into<String>) -> Self {
        self.naming_field = Some(field.into());
        self
    }

    /// Mark the DocType a Single (`is_single`): exactly one document, no list.
    #[must_use]
    pub fn single(mut self) -> Self {
        self.is_single = true;
        self
    }

    /// Choose which view the Desk opens for this doctype's documents. The
    /// route stays `/desk/:module/:doctype/:name`; only the presentation
    /// changes, so the engine never has to know about views.
    #[must_use]
    pub fn view(mut self, view: DocView) -> Self {
        self.view = view;
        self
    }

    /// Declare the doctype's documents dashboards opened through `binding`,
    /// and set [`DocView::Dashboard`] in one step (the two are meaningless
    /// apart: a binding without the view is never read).
    #[must_use]
    pub fn dashboard_view(mut self, binding: DashboardView) -> Self {
        self.view = DocView::Dashboard;
        self.dashboard = binding;
        self
    }

    /// The wrapper form of [`Self::dashboard_view`]: `field` is a `Link` whose
    /// value names the dashboard document to render.
    #[must_use]
    pub fn dashboard_link(self, field: &str) -> Self {
        self.dashboard_view(DashboardView::link_field(field))
    }

    /// Mark the DocType submittable (draft → submitted → cancelled).
    #[must_use]
    pub fn submittable(mut self) -> Self {
        self.is_submittable = true;
        self
    }

    /// Mark the DocType a tree (nested set).
    #[must_use]
    pub fn tree(mut self) -> Self {
        self.is_tree = true;
        self
    }

    /// Mark the DocType virtual (no table of its own).
    #[must_use]
    pub fn virtual_doctype(mut self) -> Self {
        self.is_virtual = true;
        self
    }

    /// Mark the DocType a child table (only ever embedded in a parent).
    #[must_use]
    pub fn child_table(mut self) -> Self {
        self.is_child_table = true;
        self
    }

    /// Hide from workspace lists and global search.
    #[must_use]
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Keep the DocType visible but read-only from the Desk.
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Offer the quick-entry modal for this DocType.
    #[must_use]
    pub fn quick_entry(mut self) -> Self {
        self.quick_entry = true;
        self
    }

    /// Record every write in the version log.
    #[must_use]
    pub fn track_changes(mut self) -> Self {
        self.track_changes = true;
        self
    }

    /// Allow documents of this type to be deleted.
    #[must_use]
    pub fn allow_delete(mut self) -> Self {
        self.allow_delete = true;
        self
    }

    /// Allow renaming a document.
    #[must_use]
    pub fn allow_rename(mut self) -> Self {
        self.allow_rename = true;
        self
    }

    /// Allow duplicating a document.
    #[must_use]
    pub fn allow_copy(mut self) -> Self {
        self.allow_copy = true;
        self
    }

    /// Allow importing rows into this DocType.
    #[must_use]
    pub fn allow_import(mut self) -> Self {
        self.allow_import = true;
        self
    }

    /// Allow exporting rows of this DocType.
    #[must_use]
    pub fn allow_export(mut self) -> Self {
        self.allow_export = true;
        self
    }

    /// Allow printing a document.
    #[must_use]
    pub fn allow_print(mut self) -> Self {
        self.allow_print = true;
        self
    }

    /// Allow emailing a document.
    #[must_use]
    pub fn allow_email(mut self) -> Self {
        self.allow_email = true;
        self
    }

    /// Allow attaching files to a document.
    #[must_use]
    pub fn allow_attach(mut self) -> Self {
        self.allow_attach = true;
        self
    }

    /// Show in the list view.
    #[must_use]
    pub fn in_list_view(mut self) -> Self {
        self.in_list_view = true;
        self
    }

    /// Show in global search.
    #[must_use]
    pub fn in_global_search(mut self) -> Self {
        self.in_global_search = true;
        self
    }

    /// Show in the calendar view.
    #[must_use]
    pub fn in_calendar(mut self) -> Self {
        self.in_calendar = true;
        self
    }

    /// Show in the kanban view.
    #[must_use]
    pub fn in_kanban(mut self) -> Self {
        self.in_kanban = true;
        self
    }

    /// Offer the read-only report view in the Desk's view switcher.
    #[must_use]
    pub fn in_report_view(mut self) -> Self {
        self.in_report_view = true;
        self
    }

    /// Show in the tree view (the parent field is declared separately with
    /// [`Self::tree_parent_field`]).
    #[must_use]
    pub fn in_tree_view(mut self) -> Self {
        self.in_tree_view = true;
        self
    }

    /// Show in the Gantt view (the dates are declared with
    /// [`Self::gantt_start_field`] and [`Self::gantt_end_field`]).
    #[must_use]
    pub fn in_gantt(mut self) -> Self {
        self.in_gantt = true;
        self
    }

    /// Which collection view opens at `/desk/:module/:doctype`.
    #[must_use]
    pub fn default_view(mut self, view: CollectionView) -> Self {
        self.default_view = Some(view);
        self
    }

    /// Group the kanban board by `field` (a multi-value `Select` or a `Link`).
    #[must_use]
    pub fn kanban_group_field(mut self, field: impl Into<String>) -> Self {
        self.kanban_group_field = Some(field.into());
        self
    }

    /// Fields shown on each kanban card, in order.
    #[must_use]
    pub fn kanban_card_fields(
        mut self,
        fields: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.kanban_card_fields = fields.into_iter().map(Into::into).collect();
        self
    }

    /// End-date field of the calendar (the start is [`Self::date_field`]).
    #[must_use]
    pub fn calendar_end_field(mut self, field: impl Into<String>) -> Self {
        self.calendar_end_field = Some(field.into());
        self
    }

    /// Title field the calendar cards use.
    #[must_use]
    pub fn calendar_title_field(mut self, field: impl Into<String>) -> Self {
        self.calendar_title_field = Some(field.into());
        self
    }

    /// The `Link` to this same doctype that nests the tree.
    #[must_use]
    pub fn tree_parent_field(mut self, field: impl Into<String>) -> Self {
        self.tree_parent_field = Some(field.into());
        self
    }

    /// Title field the tree rows use.
    #[must_use]
    pub fn tree_label_field(mut self, field: impl Into<String>) -> Self {
        self.tree_label_field = Some(field.into());
        self
    }

    /// Levels expanded when the tree first opens.
    #[must_use]
    pub fn tree_expanded_levels(mut self, levels: u8) -> Self {
        self.tree_expanded_levels = Some(levels);
        self
    }

    /// Start-date field of the Gantt bars.
    #[must_use]
    pub fn gantt_start_field(mut self, field: impl Into<String>) -> Self {
        self.gantt_start_field = Some(field.into());
        self
    }

    /// End-date field of the Gantt bars.
    #[must_use]
    pub fn gantt_end_field(mut self, field: impl Into<String>) -> Self {
        self.gantt_end_field = Some(field.into());
        self
    }

    /// The `Link` to this same doctype that names a bar's predecessor.
    #[must_use]
    pub fn gantt_dependency_field(mut self, field: impl Into<String>) -> Self {
        self.gantt_dependency_field = Some(field.into());
        self
    }

    /// The `Percent`/`Int`/`Float` field (0–100) that fills a bar.
    #[must_use]
    pub fn gantt_progress_field(mut self, field: impl Into<String>) -> Self {
        self.gantt_progress_field = Some(field.into());
        self
    }

    /// Append a field.
    #[must_use]
    pub fn field(mut self, field: FieldMeta) -> Self {
        self.fields.push(field);
        self
    }

    /// Append several fields.
    #[must_use]
    pub fn fields(mut self, fields: impl IntoIterator<Item = FieldMeta>) -> Self {
        self.fields.extend(fields);
        self
    }

    /// Append a permission rule.
    #[must_use]
    pub fn permission(mut self, permission: impl Into<PermissionRule>) -> Self {
        self.permissions.push(permission.into());
        self
    }

    /// Append several permission rules.
    #[must_use]
    pub fn permissions(
        mut self,
        permissions: impl IntoIterator<Item = impl Into<PermissionRule>>,
    ) -> Self {
        self.permissions
            .extend(permissions.into_iter().map(Into::into));
        self
    }

    /// Append a secondary index.
    #[must_use]
    pub fn index(mut self, index: IndexMeta) -> Self {
        self.indexes.push(index);
        self
    }

    /// Default ordering: field and direction (`"asc"`/`"desc"`).
    #[must_use]
    pub fn sort(mut self, field: impl Into<String>, order: impl Into<String>) -> Self {
        self.sort_field = Some(field.into());
        self.sort_order = Some(order.into());
        self
    }

    /// Field used as the document title.
    #[must_use]
    pub fn title_field(mut self, field: impl Into<String>) -> Self {
        self.title_field = Some(field.into());
        self
    }

    /// Field used as the image/avatar.
    #[must_use]
    pub fn image_field(mut self, field: impl Into<String>) -> Self {
        self.image_field = Some(field.into());
        self
    }

    /// Field used as the calendar date.
    #[must_use]
    pub fn date_field(mut self, field: impl Into<String>) -> Self {
        self.date_field = Some(field.into());
        self
    }

    /// Field used as the kanban colour.
    #[must_use]
    pub fn color_field(mut self, field: impl Into<String>) -> Self {
        self.color_field = Some(field.into());
        self
    }
}
