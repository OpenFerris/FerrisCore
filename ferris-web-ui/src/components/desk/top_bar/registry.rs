//! The view-owned slots the top bar reads: actions, title, status, chrome,
//! owner and doctype. Each one is provided once for the desk shell subtree
//! (see `app.rs`) and written by whichever view is on screen.

use leptos::prelude::*;

use crate::components::ui::badge::BadgeVariant;

use super::specs::TopBarAction;

// ---------------------------------------------------------------------------
// Action registry
// ---------------------------------------------------------------------------

/// The registry type: a reactive vector of actions, provided once for the
/// desk shell subtree and read by the top bar.
pub type TopBarActions = RwSignal<Vec<TopBarAction>>;

/// Provide the registry for the desk shell subtree. Views publish into it via
/// [`use_top_bar_actions`]; the top bar reads it via [`super::TopBar`].
pub fn provide_top_bar_actions() {
    provide_context::<TopBarActions>(RwSignal::new(Vec::new()));
}

/// Read the action registry (the current view replaces its contents).
///
/// Must be called while the reactive owner is alive, i.e. from the component
/// body or an effect. `on_cleanup` runs *after* the owner is torn down, so a
/// context lookup there finds no owner and panics: read the registry up front
/// and keep the returned signal, which stays usable (and degrades to a no-op
/// once disposed) no matter when it is written.
pub fn use_top_bar_actions() -> TopBarActions {
    expect_context::<TopBarActions>()
}

// ---------------------------------------------------------------------------
// Title
// ---------------------------------------------------------------------------

/// The per-view title beside the breadcrumbs (the dashboard's label).
///
/// The bar draws route crumbs, which name *where* the view is; a view whose
/// own name matters publishes it here and clears it when it is torn down.
///
/// A newtype around the signal, like [`TopBarDoctype`]: Leptos keys contexts
/// by `TypeId`, and a bare `RwSignal<Option<String>>` would be silently
/// swapped out by any other such context provided in the same subtree.
#[derive(Clone, Copy)]
pub struct TopBarTitle(RwSignal<Option<String>>);

/// Provide the title slot for the desk shell subtree.
pub fn provide_top_bar_title() {
    provide_context(TopBarTitle(RwSignal::new(None)));
}

/// Read the title slot. Same lifetime rule as [`use_top_bar_actions`]: an
/// `None` means "no title", which is also what a torn-down view leaves.
pub fn use_top_bar_title() -> RwSignal<Option<String>> {
    expect_context::<TopBarTitle>().0
}

// ---------------------------------------------------------------------------
// Status indicator
// ---------------------------------------------------------------------------

/// A view-owned status indicator (Frappe's page indicator): a label and the
/// badge variant to paint it with.
///
/// The top bar draws route breadcrumbs, so a view that needs an indicator
/// beside them — the form's "No guardado", "Enviado", … — publishes one here
/// and clears it when it is torn down.
#[derive(Clone)]
pub struct TopBarStatus {
    pub label: String,
    pub variant: BadgeVariant,
}

/// The status registry: at most one indicator at a time.
pub type TopBarStatusSlot = RwSignal<Option<TopBarStatus>>;

/// Provide the status slot for the desk shell subtree.
pub fn provide_top_bar_status() {
    provide_context::<TopBarStatusSlot>(RwSignal::new(None));
}

/// Read the status slot. Same lifetime rule as [`use_top_bar_actions`]: grab it
/// while the owner is alive, so `on_cleanup` can still clear it.
pub fn use_top_bar_status() -> TopBarStatusSlot {
    expect_context::<TopBarStatusSlot>()
}

// ---------------------------------------------------------------------------
// Chrome
// ---------------------------------------------------------------------------

/// Layout the current view asks of the shared top bar.
///
/// Both flags default to the bar the Desk has always drawn (inside the content
/// column, breadcrumbs and title on one row):
///
/// - [`TopBarChromeSpec::fullscreen`] — the dashboard is in full-screen mode.
///   `app.rs` reads it to drop the sidebar, and the bar itself keeps only the
///   full-screen toggle plus, when [`TopBarChromeSpec::title`] allows it, the
///   view title: every other action is hidden, because the screen belongs to
///   the board.
/// - [`TopBarChromeSpec::title`] — whether the dashboard's title survives that
///   mode. It is a board setting (`title_full_screen`), so a dashboard can opt
///   out and leave the bar to the toggle alone.
///
/// A board publishes these from its resolved style (Desk default → dashboard
/// override) plus its own full-screen signal, so the configuration drawer
/// previews them live.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopBarChromeSpec {
    /// The dashboard is in full-screen mode.
    pub fullscreen: bool,
    /// Show the dashboard title while it is in full-screen mode.
    pub title: bool,
}

/// The chrome slot: one spec for the whole bar, because the two flags describe
/// one layout and a view publishes them together.
///
/// A newtype around the signal, like [`TopBarTitle`]: Leptos keys contexts by
/// `TypeId`, and a bare `RwSignal<TopBarChromeSpec>` would be silently swapped
/// out by any other such context provided in the same subtree.
#[derive(Clone, Copy)]
pub struct TopBarChrome(RwSignal<TopBarChromeSpec>);

/// Provide the chrome slot for the desk shell subtree.
pub fn provide_top_bar_chrome() {
    provide_context(TopBarChrome(RwSignal::new(TopBarChromeSpec::default())));
}

/// Read the chrome slot. Same lifetime rule as [`use_top_bar_actions`]: grab it
/// while the owner is alive, so `on_cleanup` can still restore the plain bar.
pub fn use_top_bar_chrome() -> RwSignal<TopBarChromeSpec> {
    expect_context::<TopBarChrome>().0
}

// ---------------------------------------------------------------------------
// Owner
// ---------------------------------------------------------------------------

/// Id the form view publishes while it owns the shared top bar.
pub const TOP_BAR_OWNER_FORM: &str = "form";

/// The id of the view that currently owns the shared top bar, or `None`.
///
/// The shell uses it to mount **view-scoped** OpenUI anchors — the form's
/// `form.toolbar.left`/`right` — only while that view is on screen. The anchors
/// are doctype-filtered but not route-filtered, so mounting them from the shell
/// unconditionally would also paint them on the *list* page of the same
/// doctype.
///
/// Only the form sets it, so there is no write race: whichever order teardown
/// and mount run in, the list route never writes and the value can only be
/// `None` or [`TOP_BAR_OWNER_FORM`].
pub type TopBarOwner = RwSignal<Option<&'static str>>;

/// Provide the owner slot for the desk shell subtree.
pub fn provide_top_bar_owner() {
    provide_context::<TopBarOwner>(RwSignal::new(None));
}

/// Read the owner slot. Same lifetime rule as [`use_top_bar_actions`].
pub fn use_top_bar_owner() -> TopBarOwner {
    expect_context::<TopBarOwner>()
}

// ---------------------------------------------------------------------------
// Doctype
// ---------------------------------------------------------------------------

/// The doctype the owning view is showing, for the view-scoped OpenUI anchors
/// (`form.toolbar.left`/`right`).
///
/// A **registry, not a route param**: the top bar renders as a sibling of
/// `<Routes>` (see `app.rs`), so it sits outside any matched `<Route>` and
/// `use_params_map()` panics there — "Tried to access params outside the context
/// of a matched `<Route>`". The view that owns the bar is the one that knows its
/// doctype, and it writes this in the same breath as [`TopBarOwner`].
///
/// A newtype around the signal, not a bare `RwSignal<String>`: Leptos keys
/// contexts by `TypeId`, and a bare `RwSignal<String>` would be silently
/// swapped out by any other such context provided in the same subtree.
#[derive(Clone, Copy)]
pub struct TopBarDoctype(RwSignal<String>);

/// Provide the doctype slot for the desk shell subtree.
pub fn provide_top_bar_doctype() {
    provide_context(TopBarDoctype(RwSignal::new(String::new())));
}

/// Read the doctype slot. Same lifetime rule as [`use_top_bar_actions`]: an
/// empty string means "no doctype", which is also what a torn-down view leaves.
pub fn use_top_bar_doctype() -> RwSignal<String> {
    expect_context::<TopBarDoctype>().0
}
