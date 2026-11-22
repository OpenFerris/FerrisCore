//! Normalization and validation of workspace declarations.

use super::specs::{DEFAULT_FOLDER_ICON, WorkspaceFolderSpec, WorkspaceItemSpec, WorkspaceSpec};

/// Normalize a declaration and collect validation warnings.
///
/// Shared by [`register_app_workspaces`](crate::workspaces::register_app_workspaces)
/// and [`WorkspaceSet::validate`](crate::workspaces::WorkspaceSet::validate) so
/// the warnings a test sees are exactly the warnings registration emits.
pub(super) fn normalize_declarations(
    owner: &str,
    folders: Vec<WorkspaceFolderSpec>,
    workspaces: Vec<WorkspaceSpec>,
) -> (Vec<WorkspaceFolderSpec>, Vec<WorkspaceSpec>, Vec<String>) {
    let mut warnings = Vec::new();

    let mut kept_folders: Vec<WorkspaceFolderSpec> = Vec::with_capacity(folders.len());
    for mut folder in folders {
        let name = folder.name.trim().to_string();
        if name.is_empty() {
            warnings.push(format!("folder (owner `{owner}`): empty name — dropped"));
            continue;
        }
        if kept_folders.iter().any(|f| f.name == name) {
            warnings.push(format!(
                "folder `{name}` (owner `{owner}`): duplicate name — last declaration wins"
            ));
            kept_folders.retain(|f| f.name != name);
        }
        folder.name = name;
        if folder.label.trim().is_empty() {
            folder.label = humanize(&folder.name);
        }
        if folder
            .icon
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            folder.icon = Some(DEFAULT_FOLDER_ICON.to_string());
        }
        kept_folders.push(folder);
    }

    let mut kept_workspaces: Vec<WorkspaceSpec> = Vec::with_capacity(workspaces.len());
    for mut spec in workspaces {
        let name = spec.name.trim().to_string();
        if name.is_empty() {
            warnings.push(format!("workspace (owner `{owner}`): empty name — dropped"));
            continue;
        }
        if kept_workspaces.iter().any(|w| w.name == name) {
            warnings.push(format!(
                "workspace `{name}` (owner `{owner}`): duplicate name — last declaration wins"
            ));
            kept_workspaces.retain(|w| w.name != name);
        }
        spec.name = name;
        if spec.label.trim().is_empty() {
            let base = spec.module.trim().split(':').next().unwrap_or("").trim();
            spec.label = if base.is_empty() {
                humanize(&spec.name)
            } else {
                humanize(base)
            };
        }
        if spec.module.trim().is_empty() {
            spec.module = owner.to_string();
        }
        dead_item_warnings(&spec.name, owner, &spec.items, &mut warnings);
        kept_workspaces.push(spec);
    }

    (kept_folders, kept_workspaces, warnings)
}

/// Warn about a leaf item with neither a target (doctype/route/url) nor a label.
fn dead_item_warnings(
    workspace: &str,
    owner: &str,
    items: &[WorkspaceItemSpec],
    warnings: &mut Vec<String>,
) {
    for item in items {
        if !item.children.is_empty() {
            dead_item_warnings(workspace, owner, &item.children, warnings);
            continue;
        }
        let has_target = item.doctype.is_some() || item.route.is_some() || item.href.is_some();
        if !has_target && item.label.trim().is_empty() {
            warnings.push(format!(
                "workspace `{workspace}` (owner `{owner}`): an item has no \
                 doctype/route/url and no label — dead link"
            ));
        }
    }
}

/// Humanize an identifier ("sales_order" -> "Sales Order").
fn humanize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap = true;
    for ch in s.chars() {
        if ch == '_' || ch == '-' {
            out.push(' ');
            cap = true;
        } else if cap {
            out.extend(ch.to_uppercase());
            cap = false;
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out.trim().to_string()
}
