//! Desk chrome classes: the framework top bar, the login gate, the internal
//! desk sidebar and the folder accents of the desk grid.

/// Top bar container.
pub const TOPBAR: &str = "ferris-topbar";

/// Login gate: the brand mark square.
pub const LOGIN_LOGO: &str = "ferris-login__logo";
/// Login gate: the validation error banner.
pub const LOGIN_ERROR: &str = "ferris-login__error";
/// Login gate: the submit button.
pub const LOGIN_SUBMIT: &str = "ferris-login__submit";

/// Internal desk sidebar: the panel behind the nav.
pub const DESK_SIDEBAR_PANEL: &str = "ferris-desk-sidebar__panel";
/// Internal desk sidebar: a nav row (button or link).
pub const DESK_SIDEBAR_ITEM: &str = "ferris-desk-sidebar__item";
/// Internal desk sidebar: the nav row for the current route.
pub const DESK_SIDEBAR_ITEM_ACTIVE: &str = "ferris-desk-sidebar__item--active";

/// Desk folder tile: the muted accent worn when the folder has no colour.
pub const FOLDER_DEFAULT: &str = "ferris-desk-folder--default";
/// Desk folder accent: a neutral slate.
pub const FOLDER_SLATE: &str = "ferris-desk-folder--slate";
/// Desk folder accent: blue.
pub const FOLDER_BLUE: &str = "ferris-desk-folder--blue";
/// Desk folder accent: emerald.
pub const FOLDER_EMERALD: &str = "ferris-desk-folder--emerald";
/// Desk folder accent: amber.
pub const FOLDER_AMBER: &str = "ferris-desk-folder--amber";
/// Desk folder accent: rose.
pub const FOLDER_ROSE: &str = "ferris-desk-folder--rose";
/// Desk folder accent: violet.
pub const FOLDER_VIOLET: &str = "ferris-desk-folder--violet";
