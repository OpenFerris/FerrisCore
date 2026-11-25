//! Multi-factor authentication (MFA)

/// The largest TOTP digit count `totp-rs` accepts. The old custom truncation
/// maths supported nine; six is what the framework generates and eight is the
/// library ceiling.
pub const MAX_TOTP_DIGITS: usize = 8;

use data_encoding::BASE32_NOPAD;
use ferris_core::{AuthError, Error, Result};
use rand::{Rng, thread_rng};
use serde::{Deserialize, Serialize};

/// MFA configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfaConfig {
    pub issuer: String,
    pub totp_digits: usize,
    pub totp_period: u64,
    pub backup_codes_count: usize,
    pub backup_code_length: usize,
}

impl Default for MfaConfig {
    fn default() -> Self {
        Self {
            issuer: "Ferris".to_string(),
            totp_digits: 6,
            totp_period: 30,
            backup_codes_count: 10,
            backup_code_length: 8,
        }
    }
}

/// TOTP configuration
#[derive(Clone, Serialize, Deserialize)]
pub struct TotpConfig {
    pub secret: String,
    pub digits: usize,
    pub period: u64,
    pub algorithm: TotpAlgorithm,
}

/// `Debug` is redacted: the secret is the second factor, so a config logged by
/// a caller (or a `{:?}` in a test failure) must not print it. The parameters
/// stay, because they are what a diagnostic actually needs.
impl std::fmt::Debug for TotpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TotpConfig")
            .field("secret", &"<redacted>")
            .field("digits", &self.digits)
            .field("period", &self.period)
            .field("algorithm", &self.algorithm)
            .finish()
    }
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
    /// Build a manager, clamping the digit count into the range `totp-rs`
    /// accepts (6–8). The default is 6, which is what the enrolment endpoint
    /// generates.
    #[must_use]
    pub fn new(mut config: MfaConfig) -> Self {
        config.totp_digits = config.totp_digits.clamp(6, MAX_TOTP_DIGITS);
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
    /// the TOTP generator. Encoding it as Base64 here (as this used to) made the
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
    /// (the TOTP generator fails closed) or the system clock is before the Unix
    /// epoch.
    pub fn generate_totp(&self, config: &TotpConfig) -> Result<String> {
        crate::totp::generate(
            &config.secret,
            u8::try_from(config.digits).unwrap_or(8),
            config.period,
            algorithm(config.algorithm),
        )
        .map_err(|error| Error::Auth(AuthError::TokenInvalid(error)))
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
        let skew = u16::try_from(window.max(0)).unwrap_or(u16::MAX);
        crate::totp::verify(
            &config.secret,
            code,
            u8::try_from(config.digits).unwrap_or(8),
            config.period,
            skew,
            algorithm(config.algorithm),
        )
        .map(|matched| matched.is_some())
        .map_err(|error| Error::Auth(AuthError::TokenInvalid(error)))
    }

    /// Generate backup codes
    ///
    /// Each character is one of `0-9A-Z`: [`char::from_digit`] covers the whole
    /// `0..36` radix, so the conversion cannot fail and there is no `unwrap()`
    /// in the generation path. Letters are upper-cased to match what the
    /// authenticator UI shows.
    #[must_use]
    pub fn generate_backup_codes(&self) -> Vec<String> {
        let mut rng = thread_rng();
        let mut codes = Vec::new();

        for _ in 0..self.config.backup_codes_count {
            let code: String = (0..self.config.backup_code_length)
                .map(|_| {
                    let digit = rng.gen_range(0..36_u32);
                    char::from_digit(digit, 36)
                        .unwrap_or('0')
                        .to_ascii_uppercase()
                })
                .collect();
            codes.push(code);
        }

        codes
    }
}

fn algorithm(algorithm: TotpAlgorithm) -> totp_rs::Algorithm {
    match algorithm {
        TotpAlgorithm::Sha1 => totp_rs::Algorithm::SHA1,
        TotpAlgorithm::Sha256 => totp_rs::Algorithm::SHA256,
        TotpAlgorithm::Sha512 => totp_rs::Algorithm::SHA512,
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

    /// The seed is the second factor: printing a `TotpConfig` must not publish
    /// it, but the parameters a diagnostic needs stay visible.
    #[test]
    fn totp_config_debug_redacts_the_secret() {
        let config = MfaManager::default().generate_totp_secret();
        let debug = format!("{config:?}");

        assert!(!debug.contains(&config.secret), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
        assert!(debug.contains("digits"), "{debug}");
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
        // secret "12345678901234567890" (ASCII) = Base32, T0 = 59 s -> counter 1.
        let code = crate::totp::generate_at(
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
            8,
            30,
            algorithm(TotpAlgorithm::Sha1),
            59,
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

    /// AUTH11: an oversized `totp_digits` is clamped at construction, so the generator
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
