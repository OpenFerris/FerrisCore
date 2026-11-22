//! Naming behaviour: pattern tokens, providers and the persistent counter.
//!
//! Frappe's naming vocabulary is shared by the static series (`TASK-.#####`),
//! the dot-separated **Expression** and the braces-delimited **Expression (old
//! style)**, so the date tokens live here once instead of in each formatter.
//! The `Script` naming method has no Python engine in Ferris: a plugin
//! registers a Rust [`NamingProvider`] instead, which keeps the choice inside
//! the process and out of a scripting sandbox.
//!
//! The module is a folder because the counter store, the pattern formatters,
//! the amendment suffix and their tests are separate concerns; `mod.rs` only
//! declares and re-exports, so every existing `crate::naming::…` path resolves.

mod amend;
mod pattern;
mod store;

#[cfg(test)]
mod tests;

pub use amend::next_amend_name;
pub use pattern::{NamingProvider, format_series, register_naming_provider};
pub use store::NamingSeriesStore;

pub(crate) use pattern::{
    format_expression, format_expression_old_style, naming_provider, random_name, text,
};
