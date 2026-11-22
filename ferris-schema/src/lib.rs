//! Ferris Schema: DocType registry, the canonical Desk wire contract, and the
//! migration engine.
//!
//! This crate exists so nothing else has to be. `ferris-web` used to reach into
//! `ferris-core`, `ferris-db`, `ferris-api` and `ferris-document` directly, and
//! because all of those pull `sqlx`, a plain `cargo check` of the Desk — and
//! every wasm32 hydrate build — had to load the whole backend to typecheck a
//! handful of DTOs. The split below keeps that from happening again:
//!
//! - [`contract`] is the wire format, pure serde, no `ferris-*` dependency at
//!   all. It is always compiled and is what the browser sees.
//! - The registry, migrations and customization need a live pool, so they sit
//!   behind the default-on `native` feature and pull `ferris-core`/`ferris-db`.
//!
//! A consumer that only speaks the wire format (the Desk) depends on this crate
//! with `default-features = false` and gets [`contract`] alone.

pub mod contract;

/// The canonical fieldtype vocabulary (`Data`, `Small Text`, …): always
/// compiled, because the Desk normalizes persisted fieldtypes through it too.
pub mod fieldtype;

#[cfg(feature = "native")]
pub mod builders;
#[cfg(feature = "native")]
pub mod custom_docperm;
#[cfg(feature = "native")]
pub mod customization;
#[cfg(feature = "native")]
pub mod doctypes;
#[cfg(feature = "native")]
pub mod migration;
#[cfg(feature = "native")]
pub mod registry;
#[cfg(feature = "native")]
pub mod workspaces;

#[cfg(feature = "native")]
pub use builders::RolePermission;
#[cfg(feature = "native")]
pub use custom_docperm::{has_custom_permission, has_custom_permission_on};
#[cfg(feature = "native")]
pub use customization::{CustomField, CustomizationStore, PropertySetter};
#[cfg(feature = "native")]
pub use migration::MigrationRunner;
#[cfg(feature = "native")]
pub use registry::{
    DoctypeMeta, FieldMeta, IndexMeta, PermissionRule, SchemaRegistry, TAGS_FIELD,
    add_standard_fields, doc_actions, doc_links, dyn_meta, dyn_meta_json, effective_meta,
    get_doctype, list_doctypes, list_doctypes_metas, permission_types, register_doctype,
    standard_fields, tags_field, try_register_app_doctype, validate_app_doctype,
};
