//! The **Data** Desk workspace, declared by `ferris-api`.
//!
//! A pure declaration: the module builds the [`WorkspaceSpec`] and hands it to
//! the framework registry under this crate's owner id. No I/O, and no dependency
//! on whether every DocType it links is registered — the host's
//! `register_workspace_catalog()` invokes this after the DocType catalog
//!.
//!
//! The sidebar mirrors §6.5. `File` is the one link whose DocType this crate
//! does **not** declare: `ferris-files` owns it (§14.5 decision 4). That
//! separation is deliberate — *who declares a link* is not *who implements the
//! DocType* — and it is why this file's test hard-codes the expected links
//! instead of comparing against this crate's own metas.
#![allow(clippy::doc_markdown)]

use ferris_core::workspaces::{
    FERRIS_FOLDER, WorkspaceItemSpec as Item, WorkspaceRegistration, WorkspaceSet, WorkspaceSpec,
};

/// Registry owner: the crate id. Re-registering under it replaces this crate's
/// previous layout, so a hot reload never leaves stale links behind.
pub const OWNER: &str = "ferris-api";

/// The links the Data workspace draws, in sidebar order: the direct DocTypes,
/// then the Naming and Logs groups.
#[must_use]
pub fn data_workspace() -> WorkspaceSpec {
    WorkspaceSpec::new("data", "Data")
        .module(crate::doctypes::DATA_MODULE)
        .icon("Database")
        .folder(FERRIS_FOLDER)
        .item(Item::doctype("Data Import"))
        .item(Item::doctype("Data Export"))
        .item(Item::doctype("File"))
        .group(
            "Naming",
            vec![
                Item::doctype("Document Naming Rule"),
                Item::doctype("Document Naming Settings"),
            ],
        )
        .group(
            "Logs",
            vec![
                Item::doctype("Data Import Log"),
                Item::doctype("Prepared Report"),
            ],
        )
}

/// Register this crate's Desk workspace, returning the names and any validation
/// warnings.
///
/// Idempotent per owner (the framework replaces this owner's previous layout).
#[must_use]
pub fn register_workspaces() -> WorkspaceRegistration {
    WorkspaceSet::default()
        .workspace(data_workspace())
        .register(OWNER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_owner_registers_the_data_workspace() {
        let registration = register_workspaces();
        assert_eq!(registration.names, vec!["data"]);
        assert!(
            registration.warnings.is_empty(),
            "{:?}",
            registration.warnings
        );
    }

    #[test]
    fn data_links_the_data_doctypes_plus_the_file_link() {
        let ws = data_workspace();
        let mut linked = Vec::new();
        for item in &ws.items {
            item.collect_doctypes(&mut linked);
        }
        let mut expected = vec![
            "Data Export",
            "Data Import",
            "Data Import Log",
            "Document Naming Rule",
            "Document Naming Settings",
            "File",
            "Prepared Report",
        ];
        linked.sort_unstable();
        expected.sort_unstable();
        assert_eq!(linked, expected);

        // Every Data DocType this crate declares is linked; `File` is the only
        // link backed by another crate.
        for meta in crate::doctypes::data_doctype_metas() {
            if meta.module == crate::doctypes::DATA_MODULE {
                assert!(
                    linked.contains(&meta.name.as_str()),
                    "{} is not linked",
                    meta.name
                );
            }
        }
    }
}
