//! Desk workspaces: app-declared sidebar definitions.
//!
//! A workspace is a first-class, app-owned description of part of the Desk
//! sidebar: a labelled group of links bound to one module. The app that owns
//! the workspace decides its contents — doctype shortcuts, internal desk
//! routes, external URLs, and nested groups/dropdowns, each with an optional
//! icon key the desk maps to an SVG.
//!
//! Apps register workspaces either by calling [`register_app_workspaces`] at
//! startup, or (the normal path) by declaring a `"workspaces": [...]` array in
//! the plugin's `describe()` JSON — the framework registers them when the
//! plugin loads. A `"folders": [...]` array alongside it declares the folders
//! the desk grid groups those workspaces into, referenced by each workspace's
//! `folder`. Registration is keyed by an owner (the plugin/app id): calling it
//! again with the same owner atomically replaces that app's previous set —
//! folders and workspaces together — so hot-reloading a plugin never leaves
//! stale desk definitions behind.
//!
//! `GET /api/modules` (in `ferris-api`) merges these declarations with the
//! schema registry so existing plugins that do not declare workspaces keep
//! their auto-derived modules.
//!
//! The host files its own workspaces under [`FERRIS_FOLDER`], declared once by
//! `ferris-server`, so the desk grid groups the framework surface together
//! while apps keep their own folders.
//!
//! The store itself lives in `crate::registry::OwnerRegistry`, the shared
//! primitive also used by `ui_extensions`.

mod registry;
mod set;
mod specs;
mod validation;

pub use registry::{
    has_workspaces, register_app_workspaces, registered_folders, registered_workspaces,
};
pub use set::{WorkspaceRegistration, WorkspaceSet};
pub use specs::{
    AppWorkspaces, DEFAULT_FOLDER_ICON, FERRIS_FOLDER, WorkspaceFolderSpec, WorkspaceItemKind,
    WorkspaceItemSpec, WorkspaceSpec, ferris_folder,
};

#[cfg(test)]
mod tests;
