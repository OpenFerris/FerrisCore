//! The component class catalog.
//!
//! Every stable `.ferris-*` class emitted by a component is declared here as a
//! constant. Components reference the constants instead of string literals, so
//! a rename is a compile error rather than a silent CSS miss, and tests in
//! `ferris-web` can assert that each constant actually exists as a selector in
//! `style/components.css`.
//!
//! Naming (BEM-ish), mirrored from the CSS header:
//!
//! ```text
//! .ferris-<component>                  root
//! .ferris-<component>--<variant|size>  variant / size modifier
//! .ferris-<component>__<part>          structural part
//! ```
//!
//! The catalog grows phase by phase as components are migrated to the class
//! layer.

pub mod badge;
pub mod button;
pub mod card;
pub mod chrome;
pub mod command;
pub mod controls;
pub mod dashboard;
pub mod feedback;
pub mod overlay;
pub mod sidenav;
pub mod status;
pub mod surfaces;
pub mod table;

/// Every class in the catalog. Kept in sync with `style/components.css` by the
/// `design_system` test in `ferris-web`.
pub const ALL: &[&str] = &[
    overlay::OVERLAY,
    overlay::DIALOG_CONTENT,
    overlay::DIALOG_DESCRIPTION,
    overlay::DIALOG_CLOSE,
    overlay::SHEET_CONTENT,
    overlay::SHEET_DESCRIPTION,
    overlay::SHEET_CLOSE,
    overlay::DRAWER_CONTENT,
    overlay::DRAWER_DESCRIPTION,
    overlay::DRAWER_HANDLE,
    button::BUTTON,
    button::BUTTON_DEFAULT,
    button::BUTTON_DESTRUCTIVE,
    button::BUTTON_OUTLINE,
    button::BUTTON_SECONDARY,
    button::BUTTON_GHOST,
    button::BUTTON_ACCENT,
    button::BUTTON_LINK,
    button::BUTTON_WARNING,
    button::BUTTON_SUCCESS,
    button::BUTTON_BORDERED,
    button::BUTTON_ACTION_PROGRESS,
    badge::BADGE,
    badge::BADGE_DEFAULT,
    badge::BADGE_SECONDARY,
    badge::BADGE_ACCENT,
    badge::BADGE_MUTED,
    badge::BADGE_DESTRUCTIVE,
    badge::BADGE_OUTLINE,
    badge::BADGE_SUCCESS,
    badge::BADGE_WARNING,
    badge::BADGE_INFO,
    card::CARD,
    card::CARD_DESCRIPTION,
    feedback::ALERT,
    feedback::CALLOUT,
    feedback::CALLOUT_DEFAULT,
    feedback::CALLOUT_INFO,
    feedback::CALLOUT_WARNING,
    controls::FIELD,
    controls::INPUT,
    controls::TEXTAREA,
    controls::LABEL,
    controls::CHECKBOX,
    controls::SWITCH,
    controls::SWITCH_THUMB,
    command::COMMAND,
    command::COMMAND_DIALOG,
    command::COMMAND_INPUT,
    command::COMMAND_ITEM,
    command::COMMAND_ITEM_LINK,
    command::COMMAND_ITEM_CHECK,
    command::COMMAND_GROUP,
    command::COMMAND_GROUP_LABEL,
    command::COMMAND_DESCRIPTION,
    command::COMMAND_FOOTER,
    chrome::TOPBAR,
    chrome::LOGIN_LOGO,
    chrome::LOGIN_ERROR,
    chrome::LOGIN_SUBMIT,
    chrome::DESK_SIDEBAR_PANEL,
    chrome::DESK_SIDEBAR_ITEM,
    chrome::DESK_SIDEBAR_ITEM_ACTIVE,
    chrome::FOLDER_DEFAULT,
    chrome::FOLDER_SLATE,
    chrome::FOLDER_BLUE,
    chrome::FOLDER_EMERALD,
    chrome::FOLDER_AMBER,
    chrome::FOLDER_ROSE,
    chrome::FOLDER_VIOLET,
    sidenav::SIDENAV,
    sidenav::SIDENAV_WRAPPER,
    sidenav::SIDENAV_INSET,
    sidenav::SIDENAV_INNER,
    sidenav::SIDENAV_CONTAINER,
    sidenav::SIDENAV_MENU_SUB,
    sidenav::SIDENAV_GROUP_LABEL,
    sidenav::SIDENAV_LINK,
    sidenav::SIDENAV_MENU_BUTTON,
    sidenav::SIDENAV_MENU_BUTTON_OUTLINE,
    sidenav::SIDENAV_MENU_SUB_BUTTON,
    sidenav::SIDENAV_MENU_ACTION,
    sidenav::SIDENAV_TRIGGER,
    sidenav::SIDENAV_TOGGLE_RAIL,
    sidenav::SIDENAV_INPUT,
    table::TABLE,
    table::TABLE_CAPTION,
    table::TABLE_HEADER,
    table::TABLE_ROW,
    table::TABLE_HEAD,
    table::TABLE_FOOTER,
    surfaces::BENTO_CELL,
    surfaces::CARD_CAROUSEL,
    surfaces::CARD_CAROUSEL_INDICATOR,
    surfaces::DRAGGABLE_ZONE,
    dashboard::DASHBOARD_BOARD,
    dashboard::DASHBOARD_TILE,
    dashboard::DASHBOARD_TILE_EDITING,
    dashboard::DASHBOARD_TILE_CHROME,
    dashboard::DASHBOARD_TILE_RESIZE,
    status::STATUS_INDICATOR_DEFAULT,
    status::STATUS_INDICATOR_ACTIVE,
    status::STATUS_INDICATOR_INACTIVE,
    status::STATUS_INDICATOR_NORMAL,
];
