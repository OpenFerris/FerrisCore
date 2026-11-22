//! Handling for `Password`-typed document fields.
//!
//! A `Password` field stores a credential that must never travel in clear. In
//! Frappe the value lives in a separate `__Auth` table (`frappe/core/doctype/
//! user/user.py`) and the field reads back as a fixed mask; Ferris keeps the
//! hash in the document blob and does the same two things the engine needs:
//! hash a plaintext on **write**, and hand back a mask on **read**.
//!
//! The hasher itself is Argon2, and it lives in `ferris-auth` — which depends on
//! this crate, so it cannot be called from here directly. It is installed as a
//! process-wide hook instead, the same shape as
//! [`crate::permissions::set_global_engine`]: `ferris-auth` registers it once at
//! boot, and the document engine (also below `ferris-auth`) hashes through it.

use crate::Result;
use std::sync::OnceLock;

/// What a `Password` field reads back as, whatever it holds.
pub const PASSWORD_MASK: &str = "********";

/// Whether `value` is the mask a previous read handed out.
///
/// Frappe calls this "dummy" (`is_dummy_password`): a save that echoes the mask
/// back means "leave the stored secret alone", not "set the password to `********`".
#[must_use]
pub fn is_password_mask(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c == '*')
}

/// A plaintext → hash function, installed by the authentication crate.
type Hasher = fn(&str) -> Result<String>;

static HASHER: OnceLock<Hasher> = OnceLock::new();

/// Install the process-wide password hasher (called once at boot).
///
/// The first call wins; a later call with a **different** function pointer
/// returns `false` so a second registration cannot silently replace the hasher
/// the already-stored hashes were produced with — swapping it mid-flight would
/// keep accepting new passwords while every existing hash stopped verifying.
/// Re-installing the *same* hasher is a no-op and returns `true`, so a re-run of
/// the boot sequence (three crates in `ferris-auth` install it) cannot fail.
///
/// Same contract as [`crate::notifications::set_publisher`].
pub fn set_password_hasher(hasher: Hasher) -> bool {
    install(&HASHER, hasher)
}

/// The body of [`set_password_hasher`] over an explicit slot, so the "different
/// hasher is refused" branch is testable without touching the process-wide
/// hook (a test cannot reset a `OnceLock` another test may have filled).
fn install(slot: &OnceLock<Hasher>, hasher: Hasher) -> bool {
    match slot.get() {
        // `fn_addr_eq`, not `==`: comparing function pointers directly is
        // unpredictable (the compiler may duplicate/fold function bodies) and
        // is denied by the workspace lint set.
        Some(installed) => std::ptr::fn_addr_eq(*installed, hasher),
        None => slot.set(hasher).is_ok(),
    }
}

/// Hash a plaintext password with the installed hasher.
///
/// Fails **closed**: with no hasher installed a plaintext must not reach the
/// blob, so the write is refused rather than stored verbatim. In practice the
/// authentication crate registers the hasher during boot, before any document
/// with a `Password` field can be written.
///
/// # Errors
///
/// Returns [`crate::ConfigError::Missing`] when no hasher has been installed,
/// or whatever the installed hasher returns (the Argon2 hasher rejects a
/// password that does not meet its policy — see `ferris-auth`).
pub fn hash_password(value: &str) -> Result<String> {
    hash_with(HASHER.get(), value)
}

/// The body of [`hash_password`] with the hasher passed in, so the
/// "no hasher" branch is testable deterministically: the hook is process-wide,
/// and a test that reads it cannot know what another test installed.
fn hash_with(hasher: Option<&Hasher>, value: &str) -> Result<String> {
    match hasher {
        Some(hasher) => hasher(value),
        None => Err(crate::ConfigError::Missing(
            "password hasher (ferris-auth registers one at boot)".to_string(),
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn a_run_of_stars_is_a_mask_but_a_password_is_not() {
        assert!(is_password_mask(PASSWORD_MASK));
        assert!(is_password_mask("*"));
        assert!(!is_password_mask(""));
        assert!(!is_password_mask("hunter2"));
        // A real password that happens to *contain* stars is not a mask.
        assert!(!is_password_mask("hu*ter2"));
    }

    #[test]
    fn hashing_without_a_hasher_fails_closed() {
        // Asserted against `hash_with(None, …)` rather than the process-wide
        // hook, so the test cannot observe a hasher another test installed. The
        // previous version accepted both `Ok` and `Err`, i.e. it could never
        // fail for the behaviour it named.
        let err = hash_with(None, "hunter2").unwrap_err();
        match err {
            Error::Config(crate::ConfigError::Missing(what)) => {
                assert!(what.contains("password hasher"), "{what}");
            }
            other => panic!("expected ConfigError::Missing, got {other:?}"),
        }
    }

    #[test]
    fn a_second_different_hasher_is_refused_and_the_same_one_is_idempotent() {
        fn first(_value: &str) -> Result<String> {
            Ok("first".to_string())
        }
        fn second(_value: &str) -> Result<String> {
            Ok("second".to_string())
        }
        // A local slot rather than the process-wide hook: a `OnceLock` cannot be
        // reset, so testing through `HASHER` would depend on test order.
        let slot: OnceLock<Hasher> = OnceLock::new();
        assert!(install(&slot, first), "the first registration installs");
        assert!(install(&slot, first), "the same hasher is idempotent");
        assert!(!install(&slot, second), "a different hasher is refused");
        assert!(
            slot.get()
                .copied()
                .is_some_and(|h| matches!(h("x"), Ok(s) if s == "first")),
            "the refused registration did not replace the installed hasher"
        );
    }

    #[test]
    fn an_installed_hasher_is_used_and_its_error_propagates() {
        fn good(_value: &str) -> Result<String> {
            Ok("$argon2id$v=19$hash".to_string())
        }
        fn refusing(_value: &str) -> Result<String> {
            Err(crate::ConfigError::Missing("too short".to_string()).into())
        }
        let good: Hasher = good;
        let refusing: Hasher = refusing;
        assert_eq!(
            hash_with(Some(&good), "hunter2").unwrap(),
            "$argon2id$v=19$hash"
        );
        assert!(hash_with(Some(&refusing), "x").is_err());
    }
}
