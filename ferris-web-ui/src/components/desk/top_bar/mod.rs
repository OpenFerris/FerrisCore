//! Simplified top bar for the Desk.
//!
//! Layout: route **breadcrumbs** on the left — with an optional **title**
//! and **status indicator** the current view publishes beside them — and a
//! *generative action slot* on the right that the current view publishes into
//! (see [`provide_top_bar_actions`] / [`use_top_bar_actions`]).
//!
//! The action slot is declarative: a view declares *what* it wants (buttons,
//! dropdowns, a selection menu) and the top bar renders the matching control.
//! This keeps the desk agnostic to the app.
//!
//! The view-owned slots keep a view from having to re-implement chrome:
//!
//! - [`TopBarAction`] — the right-hand controls a view publishes.
//! - [`TopBarTitle`] — the view's own name beside the crumbs (the dashboard's
//!   label): crumbs say *where* the view is, the title says *what* it is.
//! - [`TopBarStatus`] — the badge beside the breadcrumbs (form "No guardado").
//! - [`TopBarChrome`] — whether the dashboard is in full-screen mode and
//!   whether its title survives it (the dashboard's board style).
//! - [`TopBarOwner`] — which view is on screen, so the shell can mount
//!   view-scoped OpenUI anchors (the form's `form.toolbar.*`).
//! - [`TopBarDoctype`] — the doctype the owning view is showing, so the shell
//!   can filter those anchors.
//!
//! `leading` / `trailing` are the escape hatch for raw views the caller owns:
//! `ferris-web-ui` cannot know `UiSlot` (the dependency goes the other way), so
//! the Desk hands them in.
//!
//! The file used to be a single 600+ line module; it is split by concern
//! (`registry` = the slots, `specs` = the declarative action types, `render`
//! = the bar itself) with this `mod` re-exporting everything, so every
//! existing `top_bar::X` path keeps resolving.

mod registry;
mod render;
mod specs;

pub use registry::{
    TOP_BAR_OWNER_FORM, TopBarActions, TopBarChrome, TopBarChromeSpec, TopBarDoctype, TopBarOwner,
    TopBarStatus, TopBarStatusSlot, TopBarTitle, provide_top_bar_actions, provide_top_bar_chrome,
    provide_top_bar_doctype, provide_top_bar_owner, provide_top_bar_status, provide_top_bar_title,
    use_top_bar_actions, use_top_bar_chrome, use_top_bar_doctype, use_top_bar_owner,
    use_top_bar_status, use_top_bar_title,
};
pub use render::{TopBar, TopBarMobileMenu};
pub use specs::{
    ButtonSpec, DropdownItem, DropdownSpec, FullScreenToggleSpec, SelectionMenuSpec,
    SelectionState, TopBarAction,
};

// ---------------------------------------------------------------------------
// Invoking a published action
// ---------------------------------------------------------------------------

// A `TopBarAction` carries `Callback`s owned by the view that published them,
// and a view's callbacks die with it (`Callback` is a `StoredValue`, and every
// effect run or derived-signal re-run disposes what the previous run owned).
// The slot can therefore hold a callback one teardown behind the DOM — a
// hydrated node, a handler the browser had already registered. Invoking one
// with [`Callback::run`](leptos::prelude::Callback::run) panics and takes the
// whole wasm module down ("reactive value … already been disposed"), so the
// bar invokes every published callback with `try_run` instead: a disposed
// callback is a click on a view that is already gone, which is a no-op, never
// a crash. Publishers still own their side of the contract — create the
// callbacks in the component body, not inside the effect that republishes the
// specs.
