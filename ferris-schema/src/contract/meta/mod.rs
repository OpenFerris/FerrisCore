//! DocType metadata as the Desk consumes it: the `/api/doctype/meta/{doctype}`
//! payload, and the subset a form or list view actually renders.
//!
//! This is the wasm boundary. `ferris-api` serializes the engine's real
//! [`crate::registry::DoctypeMeta`] through [`DynMeta::from_serde_meta`], and
//! the hydrate build deserializes the same JSON directly — so widening this
//! struct widens the public contract. Adding a field is safe (the server omits
//! what it does not set and every optional field defaults); renaming or
//! retyping one is a breaking change for every Desk already deployed.
//!
//! References (Frappe):
//! - fieldtypes: <https://docs.frappe.io/framework/user/en/basics/doctypes/fieldtypes>
//! - DocField properties: <https://docs.frappe.io/framework/user/en/basics/doctypes/docfield>
//! - Child/child table doctypes: <https://docs.frappe.io/framework/user/en/basics/doctypes/child-doctype>
//!
//! The shapes are grouped by concern: [`naming`], [`view`], [`dyn_meta`],
//! [`field`], [`permissions`] and [`relations`].

mod dyn_meta;
mod field;
mod naming;
mod permissions;
mod relations;
mod view;

pub use dyn_meta::DynMeta;
pub use field::{DynField, DynIndex};
pub use naming::NamingType;
pub use permissions::{
    CustomPermissionGrant, DynCustomPermission, DynPermissionRule, PermissionMatrix,
};
pub use relations::{ConnectionDocument, ConnectionGroup, Connections, DynDocAction, DynDocLink};
pub use view::{CollectionView, DashboardBinding, DashboardView, DocView, DocumentFormat};

#[cfg(test)]
mod tests;
