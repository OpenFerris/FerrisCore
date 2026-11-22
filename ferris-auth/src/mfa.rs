//! Multi-factor authentication (MFA)

/// The largest TOTP digit count the truncation maths supports: `10^digits` must
/// fit in a `u32`, and the extracted 31-bit integer is at most `2^31 - 1`
/// (AUTH11).
pub const MAX_TOTP_DIGITS: usize = 9;

use data_encoding::BASE32_NOPAD;
use ferris_core::{AuthError, Error, Result};
use hmac::{Hmac, Mac};
use rand::{Rng, thread_rng};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::crypto::constant_time_eq;

/// MFA configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfaConfig {
    pub enabled: bool,
    pub issuer: String,
    pub totp_digits: usize,
    pub totp_period: u64,
    pub backup_codes_count: usize,
    pub backup_code_length: usize,
}

impl Default for MfaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            issuer: "Ferris".to_string(),
            totp_digits: 6,
            totp_period: 30,
            backup_codes_count: 10,
            backup_code_length: 8,
        }
    }
}

/// TOTP configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotpConfig {
    pub secret: String,
    pub digits: usize,
    pub period: u64,
    pub algorithm: TotpAlgorithm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TotpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Default for TotpConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
            digits: 6,
            period: 30,
            algorithm: TotpAlgorithm::Sha1,
        }
    }
}

/// MFA manager
#[derive(Clone)]
pub struct MfaManager {
    config: MfaConfig,
}

impl MfaManager {
    /// Build a manager, clamping the TOTP digit count into the range the
    /// truncation maths supports.
    ///
    /// `hotp` computes `binary % 10^digits`: at 10 digits that exponent
    /// overflows `u32` (panic in debug, wrap in release — AUTH11), so the
    /// configured value is clamped to `1..=9` once, here, instead of relying on
    /// every caller to pass a sane config.
    #[must_use]
    pub fn new(mut config: MfaConfig) -> Self {
        config.totp_digits = config.totp_digits.clamp(1, MAX_TOTP_DIGITS);
        Self { config }
    }

    /// The configured TOTP code length.
    #[must_use]
    pub const fn totp_digits(&self) -> usize {
        self.config.totp_digits
    }

    /// The configured TOTP time step in seconds.
    #[must_use]
    pub const fn totp_period(&self) -> u64 {
        self.config.totp_period
    }

    /// Generate a new TOTP secret (RFC 4648 Base32, unpadded).
    ///
    /// Base32 is what the `secret=` parameter of an `otpauth://` URI requires
    /// and what authenticator apps decode; the same string is fed back into
    /// [`Self::hotp`]. Encoding it as Base64 here (as this used to) made the
    /// server derive a different key than every standard app, so no generated
    /// code ever verified.
    ///
    /// The secret is independent of the account, which is only ever needed to
    /// *label* the URI ([`Self::generate_totp_uri`]), so this takes no
    /// `account_name` (AUTH13).
    #[must_use]
    pub fn generate_totp_secret(&self) -> TotpConfig {
        let mut rng = thread_rng();
        let mut secret_bytes = [0u8; 20];
        rng.fill(&mut secret_bytes);
        let secret = BASE32_NOPAD.encode(&secret_bytes);

        TotpConfig {
            secret,
            digits: self.config.totp_digits,
            period: self.config.totp_period,
            algorithm: TotpAlgorithm::Sha1,
        }
    }

    /// Generate TOTP URI for QR code
    pub fn generate_totp_uri(&self, config: &TotpConfig, account_name: &str) -> String {
        let params = format!(
            "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm={}&digits={}&period={}",
            self.config.issuer,
            account_name,
            config.secret,
            self.config.issuer,
            match config.algorithm {
                TotpAlgorithm::Sha1 => "SHA1",
                TotpAlgorithm::Sha256 => "SHA256",
                TotpAlgorithm::Sha512 => "SHA512",
            },
            config.digits,
            config.period,
        );
        params
    }

    /// Generate the current TOTP code.
    ///
    /// # Errors
    ///
    /// [`AuthError::TokenInvalid`] when the stored secret is not valid Base32
    /// ([`Self::hotp`] fails closed) or the system clock is before the Unix
    /// epoch.
    pub fn generate_totp(&self, config: &TotpConfig) -> Result<String> {
        let counter = self.get_time_counter(config.period)?;
        self.hotp(&config.secret, counter, config.digits, config.algorithm)
    }

    /// Verify a TOTP code within `window` time steps either side of now.
    ///
    /// A secret that does not decode is a hard failure, not a mismatch: probing
    /// the window would otherwise accept codes derived from the empty key
    /// (AUTH9).
    ///
    /// # Errors
    ///
    /// [`AuthError::TokenInvalid`] for an undecodable secret, or when the
    /// system clock is before the Unix epoch.
    pub fn verify_totp(&self, config: &TotpConfig, code: &str, window: i64) -> Result<bool> {
        let counter = self.get_time_counter(config.period)?;

        for i in -window..=window {
            let Some(test_counter) = counter.checked_add_signed(i) else {
                continue;
            };
            let expected = self.hotp(
                &config.secret,
                test_counter,
                config.digits,
                config.algorithm,
            )?;
            if constant_time_eq(expected.as_bytes(), code.as_bytes()) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Generate backup codes
    pub fn generate_backup_codes(&self) -> Vec<String> {
        let mut rng = thread_rng();
        let mut codes = Vec::new();

        for _ in 0..self.config.backup_codes_count {
            let code: String = (0..self.config.backup_code_length)
                .map(|_| {
                    let c = rng.gen_range(0..36);
                    if c < 10 {
                        char::from_digit(c, 10).unwrap()
                    } else {
                        char::from_u32('A' as u32 + c - 10).unwrap()
                    }
                })
                .collect();
            codes.push(code);
        }

        codes
    }

    /// Seconds since the Unix epoch divided by `period`.
    ///
    /// # Errors
    ///
    /// [`AuthError::TokenInvalid`] if the system clock is before the epoch —
    /// a fallible result instead of the `unwrap` that used to panic there
    /// (AUTH11).
    fn get_time_counter(&self, period: u64) -> Result<u64> {
        let period = period.max(1);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| {
                Error::Auth(AuthError::TokenInvalid(format!(
                    "system clock is before the Unix epoch: {e}"
                )))
            })?
            .as_secs();
        Ok(now / period)
    }

    /// RFC 6238 HOTP with dynamic truncation.
    ///
    /// # Errors
    ///
    /// [`AuthError::TokenInvalid`] when `secret` is not valid Base32. This used
    /// to `unwrap_or_default()`, which turned a corrupted secret into the
    /// **empty key** and happily generated and verified codes from it (AUTH9);
    /// failing closed is the only safe behaviour for a credential.
    fn hotp(
        &self,
        secret: &str,
        counter: u64,
        digits: usize,
        algorithm: TotpAlgorithm,
    ) -> Result<String> {
        let secret_bytes = BASE32_NOPAD.decode(secret.as_bytes()).map_err(|e| {
            Error::Auth(AuthError::TokenInvalid(format!(
                "TOTP secret is not valid Base32: {e}"
            )))
        })?;
        // An *empty* decoded secret is the same hazard as an undecodable one:
        // it is a well-defined, attacker-computable key. Base32 decodes `""` to
        // zero bytes without error, so this has to be rejected explicitly
        // (AUTH9).
        if secret_bytes.is_empty() {
            return Err(Error::Auth(AuthError::TokenInvalid(
                "TOTP secret is empty".to_string(),
            )));
        }
        let counter_bytes = counter.to_be_bytes();

        let result = match algorithm {
            TotpAlgorithm::Sha1 => {
                let mut mac = Hmac::<Sha1>::new_from_slice(&secret_bytes)
                    .expect("HMAC accepts keys of any size");
                mac.update(&counter_bytes);
                mac.finalize().into_bytes().to_vec()
            }
            TotpAlgorithm::Sha256 => {
                let mut mac = Hmac::<Sha256>::new_from_slice(&secret_bytes)
                    .expect("HMAC accepts keys of any size");
                mac.update(&counter_bytes);
                mac.finalize().into_bytes().to_vec()
            }
            TotpAlgorithm::Sha512 => {
                let mut mac = Hmac::<Sha512>::new_from_slice(&secret_bytes)
                    .expect("HMAC accepts keys of any size");
                mac.update(&counter_bytes);
                mac.finalize().into_bytes().to_vec()
            }
        };

        // Dynamic truncation (last byte low nibble selects the offset).
        let offset = (result[result.len() - 1] & 0x0F) as usize;
        let binary = ((result[offset] & 0x7F) as u32) << 24
            | (result[offset + 1] as u32) << 16
            | (result[offset + 2] as u32) << 8
            | (result[offset + 3] as u32);

        // `digits` is clamped to `1..=MAX_TOTP_DIGITS` in `new`, so the exponent
        // cannot overflow (AUTH11).
        let otp = binary % 10_u32.pow(digits.min(MAX_TOTP_DIGITS) as u32);
        Ok(format!("{:0width$}", otp, width = digits))
    }
}

impl Default for MfaManager {
    fn default() -> Self {
        Self::new(MfaConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_totp_generation() {
        let manager = MfaManager::default();
        let config = manager.generate_totp_secret();

        assert!(!config.secret.is_empty());
        assert_eq!(config.digits, 6);
        assert_eq!(config.period, 30);
    }

    #[test]
    fn test_totp_verification() {
        let manager = MfaManager::default();
        let config = manager.generate_totp_secret();

        let code = manager.generate_totp(&config).expect("valid secret");
        assert!(manager.verify_totp(&config, &code, 1).unwrap());

        // Wrong code should fail
        assert!(!manager.verify_totp(&config, "000000", 1).unwrap());
    }

    #[test]
    fn test_backup_codes() {
        let manager = MfaManager::default();
        let codes = manager.generate_backup_codes();

        assert_eq!(codes.len(), 10);
        for code in &codes {
            assert_eq!(code.len(), 8);
            assert!(code.chars().all(|c| c.is_ascii_alphanumeric()));
        }
    }

    #[test]
    fn test_totp_uri() {
        let manager = MfaManager::new(MfaConfig {
            issuer: "TestApp".to_string(),
            ..Default::default()
        });
        let config = manager.generate_totp_secret();
        let uri = manager.generate_totp_uri(&config, "user@example.com");

        assert!(uri.starts_with("otpauth://totp/"));
        assert!(uri.contains("TestApp"));
        assert!(uri.contains("secret="));
    }

    /// The secret must be Base32 — the `otpauth://` contract — not Base64, and
    /// the URI must carry exactly the stored secret so an authenticator app
    /// derives the same key the server does.
    #[test]
    fn totp_secret_is_base32_and_the_uri_carries_it() {
        let manager = MfaManager::default();
        let config = manager.generate_totp_secret();

        let decoded = BASE32_NOPAD
            .decode(config.secret.as_bytes())
            .expect("generated TOTP secret is valid Base32");
        assert_eq!(decoded.len(), 20, "20 random bytes");

        let uri = manager.generate_totp_uri(&config, "user@example.com");
        assert!(uri.contains(&format!("secret={}", config.secret)), "{uri}");
    }

    /// Cross-check against RFC 6238 Appendix B (SHA-1): the code derives from a
    /// standard Base32 secret, so a real authenticator agrees with the server.
    #[test]
    fn totp_matches_the_rfc6238_sha1_vector() {
        let manager = MfaManager::default();
        // secret "12345678901234567890" (ASCII) = Base32, T0 = 59 s -> counter 1.
        let code = manager
            .hotp(
                "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
                1,
                8,
                TotpAlgorithm::Sha1,
            )
            .expect("valid Base32 secret");
        assert_eq!(code, "94287082");
    }

    /// AUTH9: a corrupted secret must fail closed, not silently degrade to the
    /// empty HMAC key. The old code used `unwrap_or_default()`, so a garbage
    /// secret produced (and verified!) codes derived from an empty key.
    #[test]
    fn a_corrupted_secret_fails_closed() {
        let manager = MfaManager::default();
        let config = TotpConfig {
            secret: "not-valid-base32!!!".to_string(),
            digits: 6,
            period: 30,
            algorithm: TotpAlgorithm::Sha1,
        };

        assert!(manager.generate_totp(&config).is_err());
        assert!(manager.verify_totp(&config, "000000", 1).is_err());

        // The empty secret is itself undecodable-to-nothing and must not verify
        // anything either.
        let empty = TotpConfig {
            secret: String::new(),
            ..config
        };
        assert!(manager.verify_totp(&empty, "000000", 1).is_err());
    }

    /// AUTH11: an oversized `totp_digits` is clamped at construction, so `hotp`
    /// cannot overflow `10^digits`.
    #[test]
    fn a_huge_digit_count_is_clamped_not_overflowed() {
        let manager = MfaManager::new(MfaConfig {
            totp_digits: 99,
            ..Default::default()
        });
        assert_eq!(manager.totp_digits(), MAX_TOTP_DIGITS);

        let config = manager.generate_totp_secret();
        let code = manager.generate_totp(&config).expect("valid secret");
        assert_eq!(code.len(), MAX_TOTP_DIGITS);
    }
}
