//! View, report, dashboard and widget DocTypes (Desk section **Build**).

use crate::contract::DashboardView;
use crate::registry::{DoctypeMeta, FieldMeta};

use super::consts::BUILD_MODULE;

use super::permissions::system_manager;

/// A saved report definition.
#[must_use]
pub fn report_meta() -> DoctypeMeta {
    DoctypeMeta::new("Report", BUILD_MODULE)
        .description("A saved report definition.")
        .title_field("report_name")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::data("report_name")
                .label("Report Name")
                .in_list_view(),
        )
        .field(FieldMeta::link("ref_doctype", "DocType").label("Reference DocType"))
        .field(
            FieldMeta::select(
                "report_type",
                "Query Report\nScript Report\nList Report\nCustom Report",
            )
            .label("Report Type")
            .default_value("Query Report")
            .in_list_view(),
        )
        .field(FieldMeta::code("query").label("Query"))
        .field(FieldMeta::code("script").label("Script").options("Python"))
        .permission(system_manager())
}

/// A dashboard definition: a route and an ordered grid of widget instances.
///
/// These documents are first-class input to the Desk, not just data: the UI
/// feed (`GET /api/ui-extensions`) merges every `Dashboard` row in beside the
/// dashboards plugins declare in `describe()`, so creating, editing or deleting
/// one changes what the Desk shows on the next fetch — the same path a plugin
/// dashboard takes. `ferris_api::dashboards` is that merge; the Desk renders
/// both through the identical 12-column grid.
///
/// The document `name` is the dashboard id (the key a per-user
/// [`crate::contract::DashboardLayout`] override is stored under); `label` is
/// the human title and `route` is the Desk URL. A row whose route is unusable
/// or already taken by another dashboard is dropped from the feed and logged,
/// exactly as the registry drops a plugin's — `ferris_api::dashboards` reuses
/// the registry's route rule so the two cannot drift.
#[must_use]
pub fn dashboard_meta() -> DoctypeMeta {
    DoctypeMeta::new("Dashboard", BUILD_MODULE)
        .description("A dashboard: a route and a grid of widget instances.")
        .title_field("label")
        .sort("modified", "desc")
        .field(
            FieldMeta::data("label")
                .label("Label")
                .in_list_view()
                .description("Title shown in the dashboard header."),
        )
        .field(
            FieldMeta::data("route")
                .label("Route")
                .in_list_view()
                .description("Desk URL the dashboard is mounted at, e.g. `/sales`."),
        )
        .field(
            FieldMeta::check("is_default")
                .label("Is Default")
                .default_value("0")
                .description("Show this dashboard at `/` when no other default is registered."),
        )
        .field(
            // The board style this dashboard declares (a `DashboardStyle`
            // object), which a user's override in the shared `Dashboard Layout`
            // row layers over. A plugin declares the same thing in
            // `describe().dashboards[].style`.
            FieldMeta::json("style")
                .label("Style")
                .description("Board appearance and geometry as a `DashboardStyle` object."),
        )
        .field(
            FieldMeta::table("widgets", "Dashboard Widget")
                .label("Widgets")
                .description("Ordered widget placements in the grid."),
        )
        // The doctype that *is* a dashboard: its own record opens as one, with
        // no binding to declare — the document name is the dashboard id and
        // the feed publishes it under the native owner. This is the native
        // case of `DocView::Dashboard`; a wrapper declares the same view with
        // a `link_field` instead.
        .dashboard_view(DashboardView::default())
        .permission(system_manager())
}

/// The `Dashboard Widget` child table: one widget placement inside a dashboard.
///
/// A row is the document-side twin of `ferris_schema::contract::WidgetInstance`
/// the registry carries: `widget` references a catalog definition (`owner.name`,
/// e.g. `host.stat`), `params` are the instance's parameter values, and
/// `span`/`col`/`row`/`rows` are its 12-column grid box. `widget_id` is the
/// instance id unique within the dashboard; the feed falls back to `row-{idx}`
/// when it is blank, so a hand-written row does not need one.
#[must_use]
pub fn dashboard_widget_meta() -> DoctypeMeta {
    DoctypeMeta::new("Dashboard Widget", BUILD_MODULE)
        .description("One widget placement in a dashboard.")
        .child_table()
        .field(
            FieldMeta::data("widget_id")
                .label("Widget Id")
                .in_list_view()
                .description("Instance id, unique within the dashboard."),
        )
        .field(
            FieldMeta::data("widget")
                .label("Widget")
                .required()
                .in_list_view()
                .description("Widget reference: `owner.name`, e.g. `host.stat`."),
        )
        .field(FieldMeta::data("label").label("Label").in_list_view())
        .field(
            FieldMeta::json("params")
                .label("Params")
                .description("Parameter values as `[[name, value], …]`."),
        )
        .field(
            // The instance's own style (a `StyleSpec` object), which layers
            // over the definition's declared default and the Desk's theme.
            FieldMeta::json("style")
                .label("Style")
                .description("Instance appearance as a `StyleSpec` object."),
        )
        .field(FieldMeta::int("span").label("Span"))
        .field(FieldMeta::int("col").label("Column"))
        .field(FieldMeta::int("row").label("Row"))
        .field(FieldMeta::int("rows").label("Rows"))
}

/// One widget definition: the runtime source of truth for the catalog.
///
/// An app's `describe().widgets` is only its **initial migration seed**. On the
/// first feed read (and whenever that seed changes) the host upserts each
/// definition here, keyed by the document name `"{app}.{widget_name}"`; after
/// that this DocType is what `/api/ui-extensions` publishes to the Desk and
/// what the layout helpers resolve. Editing a row — its label, parameters, or
/// even its JavaScript — changes the running Desk without rebuilding the app.
///
/// The fields mirror `ferris_core::ui_extensions::WidgetDefSpec` one-to-one.
/// `script`/`stylesheet`/`ssr` hold the full source; `code_path` is the app's
/// original asset path, so the host can serve the stored `script` there and
/// relative `import`s keep resolving. `seed_hash` is the fingerprint the host
/// compares against the app's seed: a matching hash means an admin may have
/// edited the row and the seed must not overwrite it.
// A DocType declaration is a field table: one field per line is the readable
// form, and splitting it would scatter a single schema across helpers.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn widget_meta() -> DoctypeMeta {
    DoctypeMeta::new("Widget", BUILD_MODULE)
        .description("One widget definition stored as data: metadata, params and source.")
        .title_field("label")
        .sort("modified", "desc")
        .field(
            FieldMeta::data("label")
                .label("Label")
                .in_list_view()
                .description("Human title shown above the widget when an instance does not override it."),
        )
        .field(
            FieldMeta::data("app")
                .label("App")
                .required()
                .in_list_view()
                .description("Publishing app; `host` for the framework's native widgets."),
        )
        .field(
            FieldMeta::data("widget_name")
                .label("Widget Name")
                .required()
                .in_list_view()
                .description("Name unique within the app (kebab-case); the global reference is `app.widget_name`."),
        )
        .field(
            FieldMeta::data("kind")
                .label("Kind")
                .in_list_view()
                .description("`stat`, `chart`, `table`, … — drives the default frame and grid span."),
        )
        .field(
            FieldMeta::data("renderer")
                .label("Renderer")
                .description("Native renderer id for a host widget (`stat`, `trend`, `table`); blank for code widgets."),
        )
        .field(
            FieldMeta::data("component")
                .label("Component")
                .description("Custom element base name; the Desk mounts `<ferris-{component}>`."),
        )
        .field(FieldMeta::text("description").label("Description"))
        .field(FieldMeta::data("requires").label("Requires"))
        .field(
            FieldMeta::json("events")
                .label("Events")
                .description("Event names the widget consumes, as a JSON array of strings."),
        )
        .field(FieldMeta::data("icon").label("Icon"))
        .field(FieldMeta::data("category").label("Category"))
        .field(FieldMeta::data("preview").label("Preview"))
        .field(
            FieldMeta::json("style")
                .label("Style")
                .description("Default appearance as a `StyleSpec` object."),
        )
        .field(FieldMeta::int("span").label("Span"))
        .field(FieldMeta::int("rows").label("Rows"))
        .field(FieldMeta::int("min_span").label("Min Span"))
        .field(FieldMeta::int("min_rows").label("Min Rows"))
        .field(
            FieldMeta::table("params", "Widget Param")
                .label("Params")
                .description("Parameters an instance may configure."),
        )
        .field(
            FieldMeta::check("enabled")
                .label("Enabled")
                .default_value("1")
                .in_list_view()
                .description("Disabled rows stay stored but are dropped from the feed; deleting a seeded row only makes the next migration recreate it."),
        )
        .field(
            FieldMeta::data("version")
                .label("Version")
                .description("Seed version the publishing app last migrated."),
        )
        .field(
            FieldMeta::data("code_path")
                .label("Code Path")
                .description("Original asset path (`ui/widgets/charts/bar.js`) the stored script is served at."),
        )
        .field(
            FieldMeta::code("script")
                .label("Script")
                .options("JavaScript")
                .description("Full JavaScript module source. Editable in the Desk."),
        )
        .field(
            FieldMeta::code("stylesheet")
                .label("Stylesheet")
                .options("CSS")
                .description("Full CSS source applied inside the widget's shadow root."),
        )
        .field(
            FieldMeta::code("ssr")
                .label("SSR")
                .options("HTML")
                .description("Optional server-rendered markup kept with the definition."),
        )
        .field(
            FieldMeta::data("seed_hash")
                .label("Seed Hash")
                .read_only()
                .hidden()
                .description("Fingerprint of the app seed; a match means the row may carry admin edits."),
        )
        .permission(system_manager())
}

/// The `Widget Param` child table: one parameter a widget exposes.
#[must_use]
pub fn widget_param_meta() -> DoctypeMeta {
    DoctypeMeta::new("Widget Param", BUILD_MODULE)
        .description("One parameter of a widget definition.")
        .child_table()
        .field(
            FieldMeta::data("param_name")
                .label("Parameter")
                .required()
                .in_list_view()
                .description("Key the instance stores; `name` is reserved by the engine."),
        )
        .field(FieldMeta::data("label").label("Label").in_list_view())
        .field(
            FieldMeta::data("value_type")
                .label("Value Type")
                .default_value("string")
                .description("`string`, `number`, `bool`, `doctype`, `json` — open vocabulary."),
        )
        .field(FieldMeta::data("default").label("Default"))
        .field(
            FieldMeta::json("options")
                .label("Options")
                .description("Allowed values for a select, as a JSON array of strings."),
        )
        .field(
            FieldMeta::check("required")
                .label("Required")
                .default_value("0"),
        )
        .field(FieldMeta::text("description").label("Description"))
        .field(FieldMeta::data("group").label("Group"))
        .field(FieldMeta::data("placeholder").label("Placeholder"))
        .field(
            FieldMeta::data("depends_on")
                .label("Depends On")
                .description("Expression deciding whether the parameter is shown, in the form renderer's depends_on language."),
        )
        .field(
            FieldMeta::check("read_only")
                .label("Read Only")
                .default_value("0"),
        )
        .field(
            FieldMeta::check("hidden")
                .label("Hidden")
                .default_value("0"),
        )
}

/// A kanban board configuration for one DocType.
#[must_use]
pub fn kanban_board_meta() -> DoctypeMeta {
    DoctypeMeta::new("Kanban Board", BUILD_MODULE)
        .description("A kanban board configuration for one DocType.")
        .title_field("name")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::data("name").label("Name").in_list_view())
        .field(
            FieldMeta::link("doc_type", "DocType")
                .label("Document Type")
                .in_list_view(),
        )
        .field(FieldMeta::data("field_name").label("Field Name"))
        .permission(system_manager())
}

/// A calendar view configuration for one DocType.
#[must_use]
pub fn calendar_view_meta() -> DoctypeMeta {
    DoctypeMeta::new("Calendar View", BUILD_MODULE)
        .description("A calendar view configuration for one DocType.")
        .title_field("name")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::data("name").label("Name").in_list_view())
        .field(
            FieldMeta::link("doc_type", "DocType")
                .label("Document Type")
                .in_list_view(),
        )
        .field(FieldMeta::data("start_date_field").label("Start Date Field"))
        .field(FieldMeta::data("end_date_field").label("End Date Field"))
        .permission(system_manager())
}

/// A saved list filter for one DocType.
#[must_use]
pub fn list_filter_meta() -> DoctypeMeta {
    DoctypeMeta::new("List Filter", BUILD_MODULE)
        .description("A saved list filter for one DocType.")
        .title_field("label")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::data("name").label("Name"))
        .field(FieldMeta::link("dt", "DocType").label("Document Type"))
        .field(
            FieldMeta::data("fieldname")
                .label("Field Name")
                .in_list_view(),
        )
        .field(FieldMeta::data("label").label("Label").in_list_view())
        .field(FieldMeta::data("filter_type").label("Filter Type"))
        .permission(system_manager())
}

/// A standalone page in the Desk.
#[must_use]
pub fn page_meta() -> DoctypeMeta {
    DoctypeMeta::new("Page", BUILD_MODULE)
        .description("A standalone page in the Desk.")
        .title_field("title")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::data("page_name")
                .label("Page Name")
                .in_list_view(),
        )
        .field(FieldMeta::data("title").label("Title").in_list_view())
        .field(FieldMeta::data("icon").label("Icon"))
        .field(FieldMeta::data("module").label("Module"))
        .field(FieldMeta::check("standard").label("Standard"))
        .permission(system_manager())
}
