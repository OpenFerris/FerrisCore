//! The view modes a doctype offers and the create affordance it grants.

use crate::meta_types::{CollectionView, DynMeta};
use crate::route::path_segment;

/// One routable view mode declared by a doctype definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Icon name for the mode selector (an `IconType` name, not [`Self::id`]).
    pub icon: &'static str,
    pub route: String,
}

/// The canonical route of a collection view.
///
/// The shape is `/desk/:module/:doctype/<kind>`, with the list living at the
/// index.
///
/// Every collection view shares this shape so the view switcher, a breadcrumb
/// and the `DocRoute` dispatcher agree on one URL. The segment is the enum's
/// own wire spelling ([`CollectionView::segment`]).
#[must_use]
pub fn collection_route(view: CollectionView, module: &str, doctype: &str) -> String {
    let base = format!("/desk/{}/{}", path_segment(module), path_segment(doctype));
    match view {
        CollectionView::List => base,
        other => format!("{base}/{}", other.segment()),
    }
}

/// The label the Desk gives a collection view (Spanish chrome).
#[must_use]
pub const fn collection_label(view: CollectionView) -> &'static str {
    match view {
        CollectionView::List => "Lista",
        CollectionView::Kanban => "Kanban",
        CollectionView::Calendar => "Calendario",
        CollectionView::Tree => "Árbol",
        CollectionView::Gantt => "Gantt",
        CollectionView::Report => "Informe",
    }
}

/// The icon the top bar and the view switcher use for a collection view.
#[must_use]
pub const fn collection_icon(view: CollectionView) -> &'static str {
    match view {
        CollectionView::List => "List",
        CollectionView::Kanban => "Kanban",
        CollectionView::Calendar => "Calendar",
        CollectionView::Tree => "FolderTree",
        CollectionView::Gantt => "ChartGantt",
        CollectionView::Report => "FileChartColumn",
    }
}

/// Whether the doctype's definition turns `view` on.
///
/// The list is always available: it is the index every doctype has, whether or
/// not it opts into `in_list_view`.
#[must_use]
pub const fn view_enabled(meta: &DynMeta, view: CollectionView) -> bool {
    if meta.is_single {
        // A Single has no collection at all.
        return matches!(view, CollectionView::List);
    }
    match view {
        CollectionView::List => true,
        CollectionView::Kanban => meta.in_kanban,
        CollectionView::Calendar => meta.in_calendar,
        CollectionView::Tree => meta.in_tree_view || meta.is_tree,
        CollectionView::Gantt => meta.in_gantt,
        CollectionView::Report => meta.in_report_view,
    }
}

/// The collection views a doctype offers, in UI order.
///
/// Derived from the view capability flags in its definition: only the modes the
/// Desk can actually render are returned. The top-bar mode selector uses this
/// list.
#[must_use]
pub fn view_modes(meta: &DynMeta, module: &str, doctype: &str) -> Vec<ViewSpec> {
    // A Single has one document and no list: its only mode is the form that
    // edits it in place. Returning a "list" mode here would offer a view that
    // cannot exist.
    if meta.is_single {
        return vec![ViewSpec {
            id: "single",
            label: "Formulario",
            icon: "FileText",
            route: collection_route(CollectionView::List, module, doctype),
        }];
    }
    let mut out = Vec::new();
    // List view is the entry point for every doctype you reach from the
    // sidebar; plugin doctypes declare it via `in_list_view`.
    if meta.in_list_view || !meta.hidden {
        out.push(ViewSpec {
            id: "list",
            label: collection_label(CollectionView::List),
            icon: collection_icon(CollectionView::List),
            route: collection_route(CollectionView::List, module, doctype),
        });
    }
    for view in [
        CollectionView::Kanban,
        CollectionView::Calendar,
        CollectionView::Tree,
        CollectionView::Gantt,
        CollectionView::Report,
    ] {
        if view_enabled(meta, view) {
            out.push(ViewSpec {
                id: view.segment(),
                label: collection_label(view),
                icon: collection_icon(view),
                route: collection_route(view, module, doctype),
            });
        }
    }
    out
}

/// The collection view that opens at `/desk/:module/:doctype`.
///
/// It is the doctype's declared `default_view` when it is enabled, else the
/// list.
/// A stale declaration (the view was turned off after being chosen) degrades
/// to the list instead of linking to a mode the doctype no longer offers.
#[must_use]
pub fn default_view(meta: &DynMeta) -> CollectionView {
    meta.default_view
        .filter(|view| view_enabled(meta, *view))
        .unwrap_or(CollectionView::List)
}

/// Whether the Desk may offer a "create" affordance for a `DocType` to the
/// given roles.
///
/// The rule lives in [`DynMeta::offers_create`], next to the `permissions`
/// payload it reads, so this crate does not re-derive the `System
/// Manager`/`Administrator` passthrough and the per-role `create` scan.
/// Authorization itself still happens in the engine.
#[must_use]
pub fn can_create(meta: &DynMeta, roles: &[String]) -> bool {
    meta.offers_create(roles)
}
