//! Document names and Desk section labels shared by every declaration in
//! [`super`].
//!
//! They live apart from the builders so a caller can name a DocType
//! (`crate::doctypes::NOTIFICATION`) without pulling in the whole metadata
//! module, and so the builders never disagree on the spelling.

pub const NOTIFICATION: &str = "Notification";
pub const EVENT: &str = "Event";
pub const NOTIFICATION_SETTINGS: &str = "Notification Settings";
pub const DESK_LAYOUT: &str = "Desk Layout";
pub const DESIRED_STATE: &str = "Desired State";
pub const DASHBOARD_LAYOUT: &str = "Dashboard Layout";
pub const DASHBOARD_SETTINGS: &str = "Dashboard Settings";
pub const TAG: &str = "Tag";

/// Desk module/section of the import/export/naming Data DocTypes.
pub const DATA_MODULE: &str = "Data";
/// Desk module/section of the framework and Desk-feed DocTypes.
pub const DESK_MODULE: &str = "Desk";
