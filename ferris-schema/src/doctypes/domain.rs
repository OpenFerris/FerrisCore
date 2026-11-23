//! The `Domain` DocType.
//!
//! Frappe uses `Domain` to restrict a workspace or shortcut to one business
//! domain. Ferris does not ship domain-specific apps, but `Workspace` and
//! `Workspace Shortcut` already carry a `restrict_to_domain` link, so the target
//! must exist rather than dangle.

use crate::registry::{DoctypeMeta, FieldMeta};

use super::consts::BUILD_MODULE;
use super::permissions::system_manager;

/// A business domain a workspace or shortcut can be restricted to.
#[must_use]
pub fn domain_meta() -> DoctypeMeta {
    DoctypeMeta::new("Domain", BUILD_MODULE)
        .description("A business domain a workspace or shortcut can be restricted to.")
        .title_field("domain")
        .sort("domain", "asc")
        .allow_delete()
        .field(
            FieldMeta::data("domain")
                .label("Domain")
                .required()
                .unique()
                .in_list_view(),
        )
        .field(FieldMeta::small_text("description").label("Description"))
        .permission(system_manager())
}
