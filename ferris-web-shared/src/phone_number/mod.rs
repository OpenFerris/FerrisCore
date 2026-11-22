//! Phone numbers and their per-country formatting rules.
//!
//! Split by concern: [`number`] owns the stored value and its parse/format
//! round trip, [`format`] the country tables the formatter reads, and `tests`
//! the format, edge-case and round-trip suites.

mod format;
mod number;

#[cfg(test)]
mod tests;

pub use format::PhoneFormat;
pub use number::PhoneNumber;
