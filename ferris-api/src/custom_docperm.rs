//! Site-specific `Custom DocPerm` overrides.
//!
//! The implementation lives in `ferris-schema` because both the API layer and
//! the WASM host enforce the same rows; this module only re-exports it so every
//! existing `crate::custom_docperm::…` path keeps resolving.

pub(crate) use ferris_schema::custom_docperm::{all, effective_permissions, rule_from_value};
