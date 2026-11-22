//! Presentational helpers derived from doctype metadata (metadata-driven front).
//!
//! Split by concern: the column/label derivation ([`columns`]), the view-mode
//! and create affordances ([`view`]) and the local filter evaluator
//! ([`filters`]). The module re-exports every item, so the historical
//! `ferris_web_shared::meta::…` paths keep resolving.

mod columns;
mod filters;
mod view;

pub use columns::*;
pub use filters::*;
pub use view::*;

#[cfg(test)]
mod tests;
