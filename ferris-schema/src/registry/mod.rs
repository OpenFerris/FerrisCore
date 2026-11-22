//! Schema registry for DocType metadata
//!
//! One process-global store plus the per-instance one tests and embedders
//! create. The split follows the registry's three jobs: [`types`] holds the
//! metadata itself, [`store`] the maps and the insert path, [`validate`] the
//! policy an app-declared `DocType` must pass (reserved names in [`reserved`]),
//! and [`standard`] the fields every `DocType` carries.

mod reserved;
mod standard;
mod store;
mod stored;
mod types;
mod validate;

pub use standard::{TAGS_FIELD, add_standard_fields, standard_fields, tags_field};
pub use store::{
    SchemaRegistry, get_doctype, list_doctypes, list_doctypes_by_module, list_doctypes_metas,
    register_doctype,
};
pub use stored::{
    doc_actions, doc_links, dyn_meta, dyn_meta_json, effective_meta, permission_types,
};
pub use types::{DoctypeMeta, FieldMeta, IndexMeta, PermissionRule};
pub use validate::{try_register_app_doctype, validate_app_doctype};

#[cfg(test)]
mod tests;
