//! The **Users** Desk workspace, declared by `ferris-auth`.
//!
//! A pure declaration: the module builds the [`WorkspaceSpec`] and hands it to
//! the framework registry under this crate's owner id. No I/O, and no dependency
//! on whether every DocType it links is registered — the host's
//! `register_workspace_catalog()` invokes this after the DocType catalog
//!.
//!
//! The sidebar mirrors §6.2: the identity DocTypes first, then profiles,
//! permissions, logs and reports.
#![allow(clippy::doc_markdown)]

use ferris_core::workspaces::{
    FERRIS_FOLDER, WorkspaceItemSpec as Item, WorkspaceRegistration, WorkspaceSet, WorkspaceSpec,
};

/// Registry owner: the crate id. Re-registering under it replaces this crate's
/// previous layout, so a hot reload never leaves stale links behind.
pub const OWNER: &str = "ferris-auth";

/// The Users workspace, in sidebar order.
#[must_use]
pub fn users_workspace() -> WorkspaceSpec {
    WorkspaceSpec::new("users", "Users")
        .module(crate::doctypes::MODULE)
        .icon("Users")
        .folder(FERRIS_FOLDER)
        .item(Item::doctype("User"))
        .item(Item::doctype("Role"))
        .item(Item::doctype("User Group"))
        .item(Item::doctype("User Invitation"))
        .group(
            "Profiles",
            vec![
                Item::doctype("Role Profile"),
                Item::doctype("Module Profile"),
                Item::doctype("User Type"),
            ],
        )
        .group("Permissions", vec![Item::doctype("User Permission")])
        .group(
            "Logs",
            vec![Item::doctype("Activity Log"), Item::doctype("Access Log")],
        )
}

/// Register this crate's Desk workspace, returning the names and any validation
/// warnings.
///
/// Idempotent per owner (the framework replaces this owner's previous layout).
#[must_use]
pub fn register_workspaces() -> WorkspaceRegistration {
    WorkspaceSet::default()
        .workspace(users_workspace())
        .register(OWNER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_owner_registers_the_users_workspace() {
        let registration = register_workspaces();
        assert_eq!(registration.names, vec!["users"]);
        assert!(
            registration.warnings.is_empty(),
            "{:?}",
            registration.warnings
        );
    }

    #[test]
    fn users_links_every_doctype_this_crate_declares() {
        let ws = users_workspace();
        let mut linked = Vec::new();
        for item in &ws.items {
            item.collect_doctypes(&mut linked);
        }
        let metas = crate::doctypes::doctype_metas();
        // `Has Role` is a child table: the crate declares and registers it as the
        // `Workspace.roles` target, but it grants nothing and a child table has no
        // list view of its own, so it is never a sidebar link. The same goes for
        // `hidden` infrastructure tables (`DocPerm`): `ferris-api` keeps hidden
        // DocTypes out of the workspace tree, so they are reached through their
        // stores, not a sidebar link.
        let mut expected: Vec<&str> = metas
            .iter()
            .filter(|meta| !meta.is_child_table && !meta.hidden)
            .map(|meta| meta.name.as_str())
            .collect();
        linked.sort_unstable();
        expected.sort_unstable();
        assert_eq!(linked, expected);
    }

    /// `Access Log` is live: `ferris-files` emits `AuditEvent::FileAccess` and the
    /// host's observability sink writes the row, so removing it from the sidebar
    /// or the catalogue would break a writer outside this crate.
    #[test]
    fn access_log_stays_linked_because_the_host_writes_it() {
        let ws = users_workspace();
        let mut linked = Vec::new();
        for item in &ws.items {
            item.collect_doctypes(&mut linked);
        }
        assert!(
            linked.contains(&"Access Log"),
            "Access Log has a host writer and must stay in the Users workspace"
        );
        assert!(
            crate::doctypes::doctype_metas()
                .iter()
                .any(|meta| meta.name == "Access Log"),
            "the declaration the host writer targets must stay registered"
        );
    }
}
