//! Debug-only redaction of secret values.

use std::fmt;

/// Marker whose `Debug` output is always `"***"`, used by the hand-written
/// `Debug` impls below so secret values can never reach logs through `{:?}`,
/// `dbg!` or `assert_eq!(..., format!("{config:?}"))` (§B4).
///
/// Serialization is deliberately left untouched: the CLI persists the merged
/// configuration to disk (redacting at display time), so `Serialize` must
/// continue emitting the real values.
pub struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"***\"")
    }
}
