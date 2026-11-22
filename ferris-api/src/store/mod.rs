//! REST document store backed by the metadata-driven [`DocumentEngine`].
//!
//! This is the **same** engine used by typed `#[doctype]` documents, so REST
//! writes run validation, lifecycle hooks, naming and (when a user is given)
//! permissions. Effective metadata (custom fields / property setters) is
//! resolved before validation.

mod crud;
mod lifecycle;
mod meta;
mod query;
mod singles;
mod validate;

pub use crud::DocStore;
pub use meta::{effective_meta, ensure_shareable, ensure_writable};
pub use validate::{validate_doctype, validate_name};

pub(crate) use meta::{document_exists, ensure_readable};

#[cfg(test)]
mod tests;
