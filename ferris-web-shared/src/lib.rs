//! Wire contract and pure helpers shared by the whole `ferris-web` crate graph.
//!
//! Nothing here touches Leptos or `web-sys`, so this crate compiles unchanged
//! for `wasm32-unknown-unknown` and for the native SSR server. That is the
//! point of the split: the DTOs, the metadata helpers and the locale tables are
//! the part of the Desk that both the browser and the server need, and keeping
//! them in a leaf crate stops a component edit from dragging them along.

#![forbid(unsafe_code)]

pub mod country;
pub mod date;
pub mod html;
pub mod markdown;
pub mod meta;
pub mod meta_types;
pub mod phone_number;
pub mod route;
pub mod theme;
pub mod types;
pub mod url;
