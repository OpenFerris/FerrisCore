//! Desk-facing server endpoints: workspaces (`/api/modules`) and global search
//! (`/api/search`).
//!
//! `modules` and `search` read the compiled schema registry, so they work for
//! any DocType the loaded app advertises. The notification feed, events feed,
//! notification settings and the user's own desk grid live in [`crate::panel`],
//! backed by the framework's core DocTypes.

mod layout;
mod modules;
mod search;
#[cfg(test)]
mod tests;

pub use layout::{
    build_layout, layout_from, load_layout, load_layout_localized, localize_layout, modules,
};
pub use modules::workspace_warnings;
pub use search::{SearchQuery, search};
