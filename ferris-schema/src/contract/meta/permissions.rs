//! Permission shapes: declared rules, custom permission types and the matrix.

use serde::{Deserialize, Serialize};

/// One role's permissions on a DocType, as declared by the doctype.
///
/// These rules drive which buttons the Desk offers. They are deliberately not
/// an authorization decision: every write is checked again by the engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DynPermissionRule {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub create: bool,
    #[serde(default)]
    pub delete: bool,
    #[serde(default)]
    pub submit: bool,
    #[serde(default)]
    pub cancel: bool,
    #[serde(default)]
    pub amend: bool,
    #[serde(default)]
    pub print: bool,
    #[serde(default)]
    pub email: bool,
    #[serde(default)]
    pub share: bool,
    #[serde(default)]
    pub report: bool,
    #[serde(default)]
    pub import: bool,
    #[serde(default)]
    pub export: bool,
    #[serde(default)]
    pub permlevel: u8,
    #[serde(default)]
    pub if_owner: bool,
    #[serde(default)]
    pub apply_to_all_doctypes: bool,
    /// Custom permission types this rule grants, by name
    /// (<https://docs.frappe.io/framework/permission-types>). Frappe's
    /// `Permission Type` rows extend the fixed read/write/… flags with
    /// app-defined rights; an older payload has no key and grants none.
    #[serde(default)]
    pub custom_permissions: Vec<String>,
}

/// One cell of the custom-permission matrix: a permission type and whether a
/// role holds it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomPermissionGrant {
    #[serde(default)]
    pub perm_type: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub granted: bool,
}

/// The role × Permission Type matrix the Role Permissions Manager renders.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionMatrix {
    #[serde(default)]
    pub doctype: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub perm_types: Vec<String>,
    /// Every `(perm_type, role)` pair; `granted` is the stored assignment.
    #[serde(default)]
    pub grants: Vec<CustomPermissionGrant>,
    /// Whether the caller may change the matrix (System Manager only).
    #[serde(default)]
    pub editable: bool,
}

/// One custom permission type declared for a doctype, from a stored
/// `Permission Type` row
/// (<https://docs.frappe.io/framework/permission-types>).
///
/// `perm_type` is the app-defined right (for example `approve`) and `role` the
/// role that grants it. Defaulted because the rows are stored data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DynCustomPermission {
    /// The custom permission name.
    #[serde(default)]
    pub perm_type: String,
    /// The role that grants it.
    #[serde(default)]
    pub role: String,
}
