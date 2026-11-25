//! One TOTP implementation for the whole crate.
//!
//! The login backend and the MFA enrolment/verification endpoints used to
//! carry separate code: one called `totp-rs`, the other reimplemented RFC 6238
//! (HMAC, truncation, Base32 decode and the time counter). Two implementations
//! of a credential algorithm drift, so both now go through this module.
//!
//! `totp-rs` accepts 6–8 digits; [`crate::MfaManager`] clamps to that range, and
//! a malformed stored seed is a hard error rather than a silent empty key.

// The module is private; `pub(crate)` documents that the helpers are shared by
// `mfa` and `login` rather than re-exported.
#![allow(clippy::redundant_pub_crate)]

use std::time::{SystemTime, UNIX_EPOCH};

use totp_rs::{Algorithm, Builder, Secret, Totp};

/// Build a TOTP checker from a Base32 seed.
///
/// # Errors
///
/// A seed that is not Base32, or a parameter set `totp-rs` rejects.
pub(crate) fn build(
    secret: &str,
    digits: u8,
    step_seconds: u64,
    skew: u16,
    algorithm: Algorithm,
) -> Result<Totp, String> {
    if secret.trim().is_empty() {
        return Err("TOTP secret is empty".to_string());
    }
    let secret = Secret::try_from_base32(secret)
        .map_err(|error| format!("TOTP secret is not valid Base32: {error}"))?;
    Builder::new()
        .with_algorithm(algorithm)
        .with_digits(digits.clamp(6, 8))
        .with_secret(secret)
        .with_step_duration(step_seconds.max(1))
        .with_skew(skew)
        .build()
        .map_err(|error| format!("TOTP parameters are unusable: {error}"))
}

/// The current Unix time, as a TOTP step source.
fn now_unix() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))
}

/// The code for `secret` at `time` (Unix seconds).
///
/// # Errors
///
/// A malformed seed, an impossible parameter set or a time before the epoch.
pub(crate) fn generate_at(
    secret: &str,
    digits: u8,
    step_seconds: u64,
    algorithm: Algorithm,
    time: u64,
) -> Result<String, String> {
    let totp = build(secret, digits, step_seconds, 0, algorithm)?;
    Ok(totp.generate(time).to_string())
}

/// The current code for `secret`.
///
/// # Errors
///
/// The same errors as [`generate_at`], including a pre-epoch clock.
pub(crate) fn generate(
    secret: &str,
    digits: u8,
    step_seconds: u64,
    algorithm: Algorithm,
) -> Result<String, String> {
    generate_at(secret, digits, step_seconds, algorithm, now_unix()?)
}

/// Check `code` against `secret`, accepting `skew` steps either side of now.
///
/// `Ok(None)` is a plain mismatch. `Err` is a broken seed, an impossible
/// parameter set or a pre-epoch clock; the caller fails closed and logs it.
///
/// # Errors
///
/// A malformed seed, an impossible parameter set or a pre-epoch clock.
pub(crate) fn verify(
    secret: &str,
    code: &str,
    digits: u8,
    step_seconds: u64,
    skew: u16,
    algorithm: Algorithm,
) -> Result<Option<u64>, String> {
    let totp = build(secret, digits, step_seconds, skew, algorithm)?;
    Ok(totp.check(code, now_unix()?))
}
