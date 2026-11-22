//! Rendering workspaces and their sidebar links into the `/api/modules` shape.

use std::collections::BTreeMap;

use ferris_core::workspaces::{WorkspaceItemKind, WorkspaceItemSpec, WorkspaceSpec};
use ferris_schema::contract::{DeskModule, DeskSidebarEntry, pretty_identifier};

/// Legacy/module-only rendering: group the reachable DocTypes per module.
pub(super) fn auto_modules() -> Vec<DeskModule> {
    doctype_modules(|_| true)
}

/// One workspace per schema module passing `keep`, listing its reachable
/// DocTypes. Ordering is deterministic (module name, then doctype name).
pub(super) fn doctype_modules(
    keep: impl Fn(&ferris_schema::DoctypeMeta) -> bool,
) -> Vec<DeskModule> {
    let mut by_module: BTreeMap<String, Vec<DeskSidebarEntry>> = BTreeMap::new();
    for meta in ferris_schema::list_doctypes_metas() {
        if meta.hidden || meta.is_child_table || meta.is_single || meta.is_virtual {
            continue;
        }
        if !keep(&meta) {
            continue;
        }
        let name = meta.name.clone();
        let mut entry = DeskSidebarEntry::doctype_link(&name, &meta.module);
        entry.title_field = meta.title_field.clone();
        entry.image_field = meta.image_field.clone();
        // A doctype may declare its own icon; it wins over the generic link.
        if let Some(icon) = meta.icon_field.clone() {
            entry.icon = Some(icon);
        }
        by_module
            .entry(meta.module.clone())
            .or_default()
            .push(entry);
    }
    by_module
        .into_iter()
        .map(|(module, items)| DeskModule {
            name: module.clone(),
            label: pretty_identifier(&module),
            module: module.clone(),
            icon: None,
            items,
        })
        .collect()
}

/// Whether a declared sidebar item can still be rendered.
///
/// A doctype link is reachable only while its DocType is registered; a route or
/// URL needs a target; and a group is reachable when at least one of its
/// children is, so a group whose links all vanished is dropped rather than
/// opening an empty dropdown. `build_layout` uses this to keep dead links out of
/// the sidebar — the start-up message for the same declarations is
/// [`workspace_warnings`].
fn item_reaches_something(item: &WorkspaceItemSpec) -> bool {
    match item.kind() {
        WorkspaceItemKind::Doctype => item.doctype.as_deref().is_some_and(|dt| {
            ferris_schema::get_doctype(dt).is_some_and(|meta| !meta.is_child_table)
        }),
        WorkspaceItemKind::Route => item
            .route
            .as_deref()
            .is_some_and(|route| !route.trim().is_empty()),
        WorkspaceItemKind::Url => item
            .href
            .as_deref()
            .is_some_and(|href| !href.trim().is_empty()),
        WorkspaceItemKind::Group => item.children.iter().any(item_reaches_something),
    }
}

/// Declared doctype links whose DocType is not registered.
///
/// `build_layout` hides those items, so this is the start-up diagnostic the host
/// logs once (rather than on every request): a workspace declares a link the
/// desk cannot draw — a typo, or a DocType not yet implemented by its crate.
/// Resolution happens here rather than in `ferris-core` because the registry
/// lives in `ferris-schema`, which `ferris-core` cannot depend on
///.
pub fn workspace_warnings() -> Vec<String> {
    let mut warnings = Vec::new();
    for ws in ferris_core::workspaces::registered_workspaces() {
        collect_unresolved(&ws.name, &ws.items, &mut warnings);
    }
    warnings
}

fn collect_unresolved(workspace: &str, items: &[WorkspaceItemSpec], out: &mut Vec<String>) {
    for item in items {
        if !item.children.is_empty() {
            collect_unresolved(workspace, &item.children, out);
            continue;
        }
        if let Some(doctype) = item.doctype.as_deref() {
            match ferris_schema::get_doctype(doctype) {
                None => out.push(format!(
                    "workspace `{workspace}`: DocType `{doctype}` is not registered — \
                     the link is hidden"
                )),
                // A child table has no standalone list: its rows belong to a
                // parent document, so a sidebar link to one can only open an
                // empty/broken list. It is hidden like an unregistered link.
                Some(meta) if meta.is_child_table => out.push(format!(
                    "workspace `{workspace}`: DocType `{doctype}` is a child table — \
                     the link is hidden"
                )),
                Some(_) => {}
            }
        }
    }
}

/// Render one declared workspace into the `/api/modules` shape.
///
/// Only the sidebar items that still resolve are drawn: a declared link to a
/// DocType this install does not have would otherwise open an empty list (or, on
/// a Single, a form with no meta). The workspace itself stays even when every
/// item is dropped — it is still the app's grid tile and its module.
pub(super) fn workspace_module(ws: &WorkspaceSpec) -> DeskModule {
    DeskModule {
        name: ws.name.clone(),
        label: ws.label.clone(),
        module: ws.module.clone(),
        icon: ws.icon.clone(),
        items: ws
            .items
            .iter()
            .filter(|item| item_reaches_something(item))
            .map(|item| workspace_item_json(item, &ws.module))
            .collect(),
    }
}

/// Render one sidebar item (recursively for groups) into the API shape. Every
/// item carries a `kind` so the desk can render doctype links, internal
/// routes, external URLs and dropdowns from one contract.
fn workspace_item_json(item: &WorkspaceItemSpec, module: &str) -> DeskSidebarEntry {
    let mut entry = DeskSidebarEntry {
        kind: item.kind().as_str().to_string(),
        label: item.label.clone(),
        icon: item.icon.clone(),
        ..Default::default()
    };

    match item.kind() {
        WorkspaceItemKind::Doctype => {
            let doctype = item.doctype.clone().unwrap_or_default();
            entry.name = Some(doctype.clone());
            entry.doctype = Some(doctype.clone());
            entry.route = item
                .route
                .clone()
                .or_else(|| Some(format!("/desk/{module}/{doctype}")));
            entry.label = if item.label.trim().is_empty() {
                pretty_identifier(&doctype)
            } else {
                item.label.clone()
            };

            let meta = ferris_schema::get_doctype(&doctype);
            entry.title_field = meta.as_ref().and_then(|m| m.title_field.clone());
            entry.image_field = meta.as_ref().and_then(|m| m.image_field.clone());
            if entry.icon.is_none() {
                entry.icon = meta.as_ref().and_then(|m| m.icon_field.clone());
            }
        }
        WorkspaceItemKind::Url => {
            entry.href = item.href.clone();
            entry.open_in_new_tab = item.open_in_new_tab;
        }
        WorkspaceItemKind::Route => {
            entry.route = item.route.clone();
        }
        WorkspaceItemKind::Group => {
            entry.children = item
                .children
                .iter()
                .filter(|child| item_reaches_something(child))
                .map(|child| workspace_item_json(child, module))
                .collect();
        }
    }

    entry
}
