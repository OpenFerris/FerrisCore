//! Global dashboard layouts, served at
//! `GET|PUT|DELETE /api/dashboard-layout/{owner}/{dashboard}`.
//!
//! A layout is the saved arrangement of an app-declared dashboard's widgets
//! (which widgets, where on the grid, with what parameters) plus its board
//! configuration.
//!
//! **One row per dashboard, shared by every user** — this is the deliberate
//! design: a dashboard is a piece of the Desk's furniture, and editing it edits
//! it for everyone (the user's explicit decision, phase C4). The row is therefore
//! *system-owned*: it is read and written through the service principal, not
//! through the caller, so no user "owns" it and the second editor does not
//! collide with the first (the earlier per-user key existed to avoid exactly
//! that collision; the contract now makes the sharing the point, so the key is
//! `"{owner}:{dashboard}"` and the ownership question does not arise).
//!
//! The storage pattern is one document per `(owner, dashboard)`, with the whole
//! ordered widget list held in one `Code` field and the board override in
//! another — a layout is always read and written whole, so splitting the widget
//! list into rows would make reordering a multi-document operation for no
//! benefit.
//!
//! The division of labour with the registry:
//!
//! - **`GET`** answers with the *effective* layout — the saved list when one
//!   exists, else the dashboard's declared seed — so the Desk renders one shape
//!   either way. The seed comes from the registry
//!   (`ferris_core::ui_extensions::registered_ui`, the same one that feeds
//!   `/api/ui-extensions`) for a plugin dashboard, or from the `Dashboard`
//!   document ([`crate::dashboards`]) for the [`crate::dashboards::OWNER`] one.
//!   The seeded instances carry the style their definition declares, so the
//!   answer is self-contained.
//! - **`PUT`** stores the full list **and the board override** and answers with
//!   what actually got saved: references that no longer resolve in the global
//!   catalog are dropped (the Desk's palette only offers resolvable ones, but a
//!   hand-crafted payload is not trusted), duplicate instance ids collapse to
//!   the first occurrence, the geometry is normalized by [`contract::layout`],
//!   and the config's numeric fields are clamped — the same rules the Desk
//!   applies, so a saved layout cannot come back shifted.
//! - **`DELETE`** removes the saved row, returning the app's seed so the Desk
//!   can swap it in without a second request.
//!
//! Editing is the `Dashboard Layout` doctype's `create` gate: `Permission::Create`
//! is not covered by the engine's owner shortcut, so *who may edit* is decided
//! by the doctype's role policy (see `doctype::dashboard_layout_meta`) — the
//! `editable` flag in every answer is that check evaluated for the caller, the
//! authoritative twin of the client's editor toggle.

mod access;
mod handlers;
mod layout;
mod seed;
mod stored;
#[cfg(test)]
mod tests;

pub use handlers::{delete_dashboard_layout, get_dashboard_layout, put_dashboard_layout};
pub(crate) use layout::{normalize_widgets, sanitize_widgets};
