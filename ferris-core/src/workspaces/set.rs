//! The declarative accumulator and the registration outcome.

use super::registry::register_app_workspaces;
use super::specs::{WorkspaceFolderSpec, WorkspaceSpec};
use super::validation::normalize_declarations;

/// Outcome of a workspace registration: the registered workspace names plus any
/// validation warnings the caller should surface.
///
/// Warnings describe declarations that were dropped or corrected — the
/// registration itself always succeeds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceRegistration {
    /// Names of the workspaces registered under the owner, in declaration order.
    pub names: Vec<String>,
    /// Human-readable validation messages (empty names, duplicate names, dead
    /// links, …).
    pub warnings: Vec<String>,
}

/// A declarative accumulator: chain folders and workspaces, then register them
/// under an owner in one call.
///
/// ```ignore
/// WorkspaceSet::default()
///     .workspace(
///         WorkspaceSpec::new("printing", "Printing")
///             .module("Printing")
///             .icon("Printer")
///             .item(WorkspaceItemSpec::doctype("Print Format")),
///     )
///     .register("ferris-print");
/// ```
#[derive(Debug, Clone, Default)]
pub struct WorkspaceSet {
    folders: Vec<WorkspaceFolderSpec>,
    workspaces: Vec<WorkspaceSpec>,
}

impl WorkspaceSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a grid folder.
    #[must_use]
    pub fn folder(mut self, folder: WorkspaceFolderSpec) -> Self {
        self.folders.push(folder);
        self
    }

    /// Append a workspace.
    #[must_use]
    pub fn workspace(mut self, workspace: WorkspaceSpec) -> Self {
        self.workspaces.push(workspace);
        self
    }

    /// Register under `owner`, returning the names and any validation warnings.
    #[must_use]
    pub fn register(self, owner: &str) -> WorkspaceRegistration {
        register_app_workspaces(owner, self.folders, self.workspaces)
    }

    /// Apply the same validation as [`Self::register`] without touching the
    /// registry (for tests).
    #[must_use]
    pub fn validate(self, owner: &str) -> WorkspaceRegistration {
        let (_folders, workspaces, warnings) =
            normalize_declarations(owner.trim(), self.folders, self.workspaces);
        WorkspaceRegistration {
            names: workspaces.into_iter().map(|w| w.name).collect(),
            warnings,
        }
    }
}
