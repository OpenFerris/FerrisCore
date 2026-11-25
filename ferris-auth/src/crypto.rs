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

use sha2::{Digest, Sha256};

/// SHA-256 of `bytes`, as 64 lowercase hexadecimal characters.
///
/// Used for credentials that do not need a password hash: a 32-byte
/// machine-generated secret already carries far more entropy than the search
/// space a slow KDF protects, and hashing it with Argon2 on every request was
/// pure latency (AUTH12). The digest is compared with [`constant_time_eq`].
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    data_encoding::HEXLOWER.encode(&hasher.finalize())
}

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

    #[test]
    fn sha256_hex_is_lowercase_and_stable() {
        // Well-known vector: the digest of the empty input.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(sha256_hex(b"abc"), sha256_hex(b"abc"));
        assert_ne!(sha256_hex(b"abc"), sha256_hex(b"abd"));
        assert_eq!(sha256_hex(b"abc").len(), 64);
        assert!(
            sha256_hex(b"anything")
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        );
        assert!(
            !sha256_hex(b"anything")
                .bytes()
                .any(|b| b.is_ascii_uppercase())
        );
    }
}
