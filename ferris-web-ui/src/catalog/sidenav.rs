//! Sidenav classes.
//!
//! The colour tokens come from the sidebar palette (`--sidebar*`); the width
//! tokens are structural (`--sidebar-width`, `--sidebar-width-icon`).

/// Root aside (non-collapsible variant and inner surfaces inherit from it).
pub const SIDENAV: &str = "ferris-sidenav";
/// Layout wrapper that provides the sidenav context.
pub const SIDENAV_WRAPPER: &str = "ferris-sidenav-wrapper";
/// Inset content area.
pub const SIDENAV_INSET: &str = "ferris-sidenav__inset";
/// Floated inner panel.
pub const SIDENAV_INNER: &str = "ferris-sidenav__inner";
/// Fixed positioning container.
pub const SIDENAV_CONTAINER: &str = "ferris-sidenav__container";
/// Nested submenu list.
pub const SIDENAV_MENU_SUB: &str = "ferris-sidenav__menu-sub";
/// Group heading.
pub const SIDENAV_GROUP_LABEL: &str = "ferris-sidenav__group-label";
/// Menu link.
pub const SIDENAV_LINK: &str = "ferris-sidenav__link";
/// Menu button.
pub const SIDENAV_MENU_BUTTON: &str = "ferris-sidenav__menu-button";
/// Outlined menu button variant.
pub const SIDENAV_MENU_BUTTON_OUTLINE: &str = "ferris-sidenav__menu-button--outline";
/// Nested submenu link button.
pub const SIDENAV_MENU_SUB_BUTTON: &str = "ferris-sidenav__menu-sub-button";
/// Action button shown on hover (also used by the "more" trigger).
pub const SIDENAV_MENU_ACTION: &str = "ferris-sidenav__menu-action";
/// Trigger button.
pub const SIDENAV_TRIGGER: &str = "ferris-sidenav__trigger";
/// Rail that toggles the collapsed sidenav.
pub const SIDENAV_TOGGLE_RAIL: &str = "ferris-sidenav__toggle-rail";
/// Search input.
pub const SIDENAV_INPUT: &str = "ferris-sidenav__input";
