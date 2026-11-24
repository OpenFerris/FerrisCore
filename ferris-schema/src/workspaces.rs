//! The **Build** and **GEO** Desk workspaces, declared by `ferris-schema`.
//!
//! These are pure declarations: the module builds [`WorkspaceSpec`]s and hands
//! them to the framework registry under this crate's owner id. No I/O, and no
//! dependency on whether every DocType it links is registered — the host's
//! `register_workspace_catalog()` invokes this after the DocType catalog
//!.
//!
//! Build is the metadata and development surface (customization, scripting,
//! views, the Desk helpers, modules and settings); GEO is the geography
//! reference data (countries, currencies, languages, translations). Both live
//! here because `ferris-schema` owns their DocTypes.
//!
//! The Build workspace links `DocType`/`DocField`, the stored form of the
//! schema registry: `ferris-document` declares them as ordinary table-backed
//! DocTypes and the host mirrors the registry into `tabDocType` at boot
//! (`ferris_document::DoctypeStore`), so a link to them is live. This replaced
//! the earlier decision to leave the
//! DocType list registry-backed with no route.
#![allow(clippy::doc_markdown)]

use ferris_core::workspaces::{
    FERRIS_FOLDER, WorkspaceItemSpec as Item, WorkspaceRegistration, WorkspaceSet, WorkspaceSpec,
};

/// Registry owner: the crate id. Re-registering under it replaces this crate's
/// previous layout, so a hot reload never leaves stale links behind.
pub const OWNER: &str = "ferris-schema";

/// The Build workspace, in sidebar order.
#[must_use]
pub fn build_workspace() -> WorkspaceSpec {
    WorkspaceSpec::new("build", "Build")
        .module(crate::doctypes::BUILD_MODULE)
        .icon("Hammer")
        .folder(FERRIS_FOLDER)
        .group(
            "Customization",
            vec![
                Item::doctype("Customize Form"),
                Item::doctype("Custom Field"),
                Item::doctype("Property Setter"),
                Item::doctype("Custom Translation"),
                Item::doctype("Navbar Settings"),
                // Actions and Links are customizable through the same page
                // (their rows carry a `custom` flag), and permission types /
                // custom DocPerms are assigned from here.
                Item::doctype("DocType Action"),
                Item::doctype("DocType Link"),
                Item::doctype("Permission Type"),
                Item::doctype("Custom DocPerm"),
                Item::doctype("Custom HTML Block"),
                Item::doctype("DocType Layout"),
            ],
        )
        .group(
            "Scripting",
            vec![
                Item::doctype("Client Script"),
                Item::doctype("Server Script"),
            ],
        )
        .group(
            "Views",
            vec![
                Item::doctype("Report"),
                Item::doctype("Dashboard"),
                Item::doctype("Dashboard Chart"),
                Item::doctype("Number Card"),
                Item::doctype("Widget"),
                Item::doctype("Kanban Board"),
                Item::doctype("Calendar View"),
                Item::doctype("List Filter"),
                Item::doctype("Page"),
                Item::doctype("Workspace"),
            ],
        )
        .group(
            "Desk",
            vec![
                Item::doctype("ToDo"),
                Item::doctype("Event"),
                Item::doctype("Tag"),
                Item::doctype("Notification Log"),
            ],
        )
        .group(
            "Modules",
            // `DocField` is a child table: it has no standalone list, so it is
            // not a sidebar destination (its rows live inside a `DocType`).
            vec![
                Item::doctype("Module Def"),
                Item::doctype("Module Onboarding"),
                Item::doctype("DocType"),
            ],
        )
        .group(
            "Settings",
            vec![
                Item::doctype("Global Search Settings"),
                Item::doctype("Notification Settings"),
                Item::doctype("Domain"),
            ],
        )
}

/// The GEO workspace, in sidebar order.
#[must_use]
pub fn geo_workspace() -> WorkspaceSpec {
    WorkspaceSpec::new("geo", "GEO")
        .module(crate::doctypes::GEO_MODULE)
        .icon("Globe")
        .folder(FERRIS_FOLDER)
        .item(Item::doctype("Country"))
        .item(Item::doctype("Currency"))
        .item(Item::doctype("Language"))
        .item(Item::doctype("Translation"))
}

/// Register this crate's Desk workspaces, returning the names and any validation
/// warnings.
///
/// Idempotent per owner (the framework replaces this owner's previous layout).
#[must_use]
pub fn register_workspaces() -> WorkspaceRegistration {
    WorkspaceSet::default()
        .workspace(build_workspace())
        .workspace(geo_workspace())
        .register(OWNER)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every doctype the workspace links, walking the groups.
    fn linked(workspace: &WorkspaceSpec) -> Vec<&str> {
        let mut out = Vec::new();
        for item in &workspace.items {
            item.collect_doctypes(&mut out);
        }
        out
    }

    #[test]
    fn the_owner_registers_build_then_geo() {
        let registration = register_workspaces();
        assert_eq!(registration.names, vec!["build", "geo"]);
        assert!(
            registration.warnings.is_empty(),
            "{:?}",
            registration.warnings
        );
    }

    #[test]
    fn build_links_every_build_doctype_once() {
        let build = build_workspace();
        let mut got = linked(&build);
        got.sort_unstable();
        // The schema-owned Build DocTypes plus the Desk helpers `ferris-api`
        // owns (`ToDo`, `Event`, `Tag`, `Notification Log`,
        // `Notification Settings`) — the Build sidebar mirrors §6.1.
        let mut expected = vec![
            "Calendar View",
            "Client Script",
            "Custom DocPerm",
            "Custom Field",
            "Custom HTML Block",
            "Custom Translation",
            "Customize Form",
            "Dashboard",
            "Dashboard Chart",
            "DocType",
            "DocType Action",
            "DocType Layout",
            "DocType Link",
            "Domain",
            "Event",
            "Global Search Settings",
            "Kanban Board",
            "List Filter",
            "Module Def",
            "Module Onboarding",
            "Navbar Settings",
            "Notification Log",
            "Notification Settings",
            "Number Card",
            "Page",
            "Permission Type",
            "Property Setter",
            "Report",
            "Server Script",
            "Tag",
            "ToDo",
            "Widget",
            "Workspace",
        ];
        expected.sort_unstable();
        assert_eq!(got, expected);
    }

    #[test]
    fn geo_links_every_geo_doctype_once() {
        let geo = geo_workspace();
        let mut got = linked(&geo);
        got.sort_unstable();
        let metas = crate::doctypes::doctype_metas();
        let mut expected: Vec<&str> = metas
            .iter()
            .filter(|meta| meta.module == crate::doctypes::GEO_MODULE)
            .map(|meta| meta.name.as_str())
            .collect();
        expected.sort_unstable();
        assert_eq!(got, expected);
    }

    #[test]
    fn register_is_idempotent() {
        assert_eq!(register_workspaces().names, register_workspaces().names);
    }
}
