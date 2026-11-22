//! Re-exports of the metadata contract.
//!
//! `DynMeta`/`DynField` and friends are defined in
//! `ferris_schema::contract::meta` — the browser-safe mirror of the registry's
//! `DoctypeMeta`. Nothing is defined here; `ferris-web` re-exports this module
//! as `api::meta_types` so the Desk keeps its historical import paths.
//!
//! The mirror is why the wasm32 build skips the native ferris crates: the
//! Desk is typed against the mirror, not against the registry, so a
//! `wasm32-unknown-unknown` check never has to resolve `sqlx`.

pub use ferris_schema::contract::meta::{
    CollectionView, DynCustomPermission, DynDocAction, DynDocLink, DynField, DynIndex, DynMeta,
    DynPermissionRule, NamingType,
};
