//! Names an app must never declare or replace.
//!
//! The registry's origin check already protects anything the framework has
//! registered, but a plugin can be described **before**
//! `register_core_doctypes` runs in some embeddings. This static set is the
//! defense for that window: the names that carry identity, permissions and the
//! storage contract itself.

use std::collections::HashSet;
use std::sync::LazyLock;

/// Framework DocType names an app must never declare or replace.
///
/// The registry's [`Origin`] already protects anything the framework has
/// registered, but a plugin can be described **before** `register_core_doctypes`
/// runs in some embeddings. This static set is the defense for that window: the
/// names that carry identity, permissions and the storage contract itself.
/// Adding a security-critical DocType to the framework means adding it here too;
/// `registry::tests` pins the names this crate declares and the critical set.
const RESERVED_DOCTYPE_NAMES: &[&str] = &[
    "Access Log",
    "Activity Log",
    "API Key",
    "Client Script",
    "Custom DocPerm",
    "Custom Field",
    "Customize Form",
    "Dashboard",
    "Dashboard Layout",
    "Dashboard Settings",
    "Desired State",
    "DocField",
    "DocPerm",
    "DocType",
    "DocType Action",
    "DocType Link",
    "Document Naming Rule",
    "Document Naming Settings",
    "Email Account",
    "Email Domain",
    "Email Template",
    "File",
    "Has Role",
    "Module Def",
    "Module Profile",
    "Notification Log",
    "OAuth Client",
    "OAuth Provider",
    "Page",
    "Permission Inspector",
    "Permission Type",
    "Print Format",
    "Property Setter",
    "Report",
    "Role",
    "Role Permission Manager",
    "Role Profile",
    "Server Script",
    "Session",
    "Social Login Key",
    "System Settings",
    "Tag",
    "Tag Link",
    "User",
    "User Doctype Permissions",
    "User Permission",
    "User Type",
    "Version",
    "Web Form",
    "Web Page",
    "Website Script",
    "Website Settings",
    "Website Theme",
    "Workspace",
];

/// [`RESERVED_DOCTYPE_NAMES`] in the same normalized form the check uses.
pub(super) static RESERVED_DOCTYPES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    RESERVED_DOCTYPE_NAMES
        .iter()
        .map(|name| normalize_doctype_name(name))
        .collect()
});

/// The comparison key for a DocType name: whitespace removed, lowercased.
///
/// `document_table` already strips whitespace, so `ToDo` and `To Do` are one
/// table; case-folding also closes the MySQL "table names are case-insensitive"
/// gap. The stored `DocTypeMeta.name` keeps its original spelling.
pub(super) fn normalize_doctype_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}
