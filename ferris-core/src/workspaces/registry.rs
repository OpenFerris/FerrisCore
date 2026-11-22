//! The process-wide workspace store and its registration entry point.

use crate::registry::OwnerRegistry;

use super::set::WorkspaceRegistration;
use super::specs::{AppWorkspaces, WorkspaceFolderSpec, WorkspaceSpec};
use super::validation::normalize_declarations;

/// Registered layouts, in registration order.
static WORKSPACES: OwnerRegistry<AppWorkspaces> = OwnerRegistry::new();

/// Register (or replace) the desk layout owned by `owner`.
///
/// Re-calling with the same owner replaces that owner's layout **in place**, so
/// a hot reload never moves its workspaces to the end of the desk order.
/// Names that are empty after trimming are dropped; a duplicate name keeps the
/// last declaration; missing labels are humanized, missing folder icons default
/// to [`DEFAULT_FOLDER_ICON`](crate::workspaces::DEFAULT_FOLDER_ICON), and an
/// item with no target and no label is kept but reported as a dead link. An
/// empty declaration drops the owner's previous layout. Every deviation is
/// reported in [`WorkspaceRegistration::warnings`].
pub fn register_app_workspaces(
    owner: &str,
    folders: Vec<WorkspaceFolderSpec>,
    workspaces: Vec<WorkspaceSpec>,
) -> WorkspaceRegistration {
    let owner = owner.trim();
    let (kept_folders, kept_workspaces, warnings) =
        normalize_declarations(owner, folders, workspaces);
    let names: Vec<String> = kept_workspaces.iter().map(|w| w.name.clone()).collect();

    let registered = WORKSPACES.write(|all| {
        if kept_folders.is_empty() && kept_workspaces.is_empty() {
            // A reload that declares nothing removes what this owner had before.
            all.retain(|layout| layout.owner != owner);
            return false;
        }

        let layout = AppWorkspaces {
            owner: owner.to_string(),
            folders: kept_folders,
            workspaces: kept_workspaces,
        };
        // Replace in place: keeps the owner's first-appearance slot stable (§3.3).
        match all.iter().position(|layout| layout.owner == owner) {
            Some(index) => all[index] = layout,
            None => all.push(layout),
        }
        true
    });
    if registered {
        WORKSPACES.mark_registered();
    }
    WorkspaceRegistration { names, warnings }
}

/// Whether any app has registered a desk layout yet.
pub fn has_workspaces() -> bool {
    WORKSPACES.is_registered()
}

/// All registered workspaces, in stable first-appearance owner order (owner
/// tags stripped).
///
/// A hot reload of one owner replaces its entry in place, so its workspaces keep
/// their slot instead of moving to the end (§3.3).
pub fn registered_workspaces() -> Vec<WorkspaceSpec> {
    WORKSPACES.read(|all| {
        all.iter()
            .flat_map(|layout| layout.workspaces.iter().cloned())
            .collect()
    })
}

/// All registered folders, in stable first-appearance owner order (owner tags
/// stripped).
pub fn registered_folders() -> Vec<WorkspaceFolderSpec> {
    WORKSPACES.read(|all| {
        all.iter()
            .flat_map(|layout| layout.folders.iter().cloned())
            .collect()
    })
}

#[cfg(test)]
pub(super) fn reset_registry() {
    WORKSPACES.reset();
}
