//! Shared cryptographic primitives for the auth surface.
//!
//! These used to be copied per module — `constant_time_eq` lived in both
//! `session.rs` and `mfa.rs` (AUTH7) — which is the "one implementation, the
//! other delegates" rule of `AGENTS.md` §4 waiting to be broken: a fix to one
//! copy (or a subtle change to how the comparison short-circuits) would
//! silently miss the other. There is now exactly one.
//!
//! Secret-dependent comparisons belong here, not inline at the call site, so
//! that "is this comparison constant-time?" has a single answer.

/// Constant-time byte comparison.
///
/// Returns as soon as the lengths differ — length is not secret in these uses
/// (a CSRF token or TOTP code has a fixed length) — but the scan over
/// equal-length buffers runs in the same time regardless of where, if anywhere,
/// the bytes stop matching, so a mismatch does not leak how many leading bytes
/// agreed.
#[must_use]
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (l, r) in left.iter().zip(right) {
        diff |= l ^ r;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_buffers_match_and_any_difference_fails() {
        assert!(constant_time_eq(b"abc123", b"abc123"));
        assert!(!constant_time_eq(b"abc123", b"abc124"));
        assert!(!constant_time_eq(b"abc123", b"abc12"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }
}
