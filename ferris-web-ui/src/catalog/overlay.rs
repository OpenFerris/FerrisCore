//! Overlay primitives: dialog, sheet and drawer.
//!
//! Their backdrops share [`OVERLAY`]; the panels and a few colour-bearing parts
//! get their own class so a theme token or a plugin rule can retheme them.

/// Scrim shared by dialog, sheet, drawer and command overlays.
pub const OVERLAY: &str = "ferris-overlay";

pub const DIALOG_CONTENT: &str = "ferris-dialog__content";
pub const DIALOG_DESCRIPTION: &str = "ferris-dialog__description";
pub const DIALOG_CLOSE: &str = "ferris-dialog__close";

pub const SHEET_CONTENT: &str = "ferris-sheet__content";
pub const SHEET_DESCRIPTION: &str = "ferris-sheet__description";
pub const SHEET_CLOSE: &str = "ferris-sheet__close";

pub const DRAWER_CONTENT: &str = "ferris-drawer__content";
pub const DRAWER_DESCRIPTION: &str = "ferris-drawer__description";
pub const DRAWER_HANDLE: &str = "ferris-drawer__handle";
