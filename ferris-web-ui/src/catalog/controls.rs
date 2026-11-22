//! Form controls: input, textarea, label, checkbox, switch, field wrapper.

/// The label/control/error wrapper of one metadata field. Its `data-invalid`
/// state repaints every nested control, so a validation error is visible on the
/// input regardless of which widget the fieldtype resolved to.
pub const FIELD: &str = "ferris-field";

/// Text field (also drives `::placeholder`, focus and invalid states).
pub const INPUT: &str = "ferris-input";
/// Multiline text field.
pub const TEXTAREA: &str = "ferris-textarea";
/// Field label.
pub const LABEL: &str = "ferris-label";
/// Checkbox.
pub const CHECKBOX: &str = "ferris-checkbox";
/// Switch track and thumb.
pub const SWITCH: &str = "ferris-switch";
pub const SWITCH_THUMB: &str = "ferris-switch__thumb";
