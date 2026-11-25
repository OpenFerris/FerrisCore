//! The password policy: configuration, its validation, and the check a
//! human-chosen password must pass.
//!
//! Hashing and verification stay in [`super`]; this module only decides what a
//! password may be. [`PasswordConfig::validate`] also guards the Argon2 knobs,
//! so a bad `[auth]` value fails before it reaches an allocation.

use ferris_core::{ConfigError, Error, Result};
use serde::{Deserialize, Serialize};

use super::{MAX_VERIFY_PASSWORD_LEN, PasswordError};

/// Password configuration
#[allow(clippy::struct_excessive_bools)] // the four class flags are the policy surface, not a state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordConfig {
    pub min_length: usize,
    pub require_uppercase: bool,
    pub require_lowercase: bool,
    pub require_numbers: bool,
    pub require_symbols: bool,
    pub max_length: usize,
    pub argon2_memory_kib: u32,
    pub argon2_iterations: u32,
    pub argon2_parallelism: u32,
}

impl Default for PasswordConfig {
    fn default() -> Self {
        Self {
            min_length: 8,
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_symbols: false,
            max_length: 128,
            argon2_memory_kib: 65536, // 64 MB
            argon2_iterations: 3,
            argon2_parallelism: 4,
        }
    }
}

/// Lower bound for `argon2_memory_kib`: the Argon2 reference implementation's
/// smallest usable memory size (`8 * p` KiB with `p = 1`).
pub const MIN_ARGON2_MEMORY_KIB: u32 = 8;
/// Upper bound for `argon2_memory_kib` (1 GiB). A larger value is a
/// misconfiguration that would let one hash or verification exhaust the host.
pub const MAX_ARGON2_MEMORY_KIB: u32 = 1_048_576;
/// Upper bound for `argon2_iterations`; the reference default is 3.
pub const MAX_ARGON2_ITERATIONS: u32 = 64;
/// Upper bound for `argon2_parallelism`; the reference default is 4.
pub const MAX_ARGON2_PARALLELISM: u32 = 64;

impl PasswordConfig {
    /// Build the hashing policy the deployment declared in `[auth]`.
    ///
    /// Only the human-facing policy (minimum length and the required character
    /// classes) is mapped; the Argon2 cost stays at [`PasswordConfig::default`],
    /// because `[auth]` does not expose it. Both the hasher installed through
    /// [`ferris_core::password::set_password_hasher`] and the backend's
    /// rehash check must be built from this, or every login rehashes forever.
    #[must_use]
    pub fn from_auth_config(config: &ferris_core::AuthConfig) -> Self {
        Self {
            min_length: config.password_min_length,
            require_uppercase: config.password_require_uppercase,
            require_lowercase: config.password_require_lowercase,
            require_numbers: config.password_require_numbers,
            require_symbols: config.password_require_symbols,
            ..Self::default()
        }
    }

    /// Check the configuration itself, not the password.
    ///
    /// The Argon2 parameters arrive from `[auth]` in the config file, so a typo
    /// must be a clear boot/validation error instead of an allocation failure
    /// or a silently weak hash. The bounds are conservative:
    ///
    /// * `argon2_memory_kib` in [`MIN_ARGON2_MEMORY_KIB`]..=[`MAX_ARGON2_MEMORY_KIB`];
    /// * `argon2_iterations` in `1..=MAX_ARGON2_ITERATIONS` and
    ///   `argon2_parallelism` in `1..=MAX_ARGON2_PARALLELISM`;
    /// * `argon2_memory_kib` of at least 8 KiB per lane (`8 * argon2_parallelism`),
    ///   which is the Argon2 minimum and what `Params::new` enforces;
    /// * `min_length` at least 1 and `max_length` between `min_length` and
    ///   [`MAX_VERIFY_PASSWORD_LEN`], so every password the policy accepts can
    ///   also be verified (the verifier refuses longer input, see
    ///   [`super::verify_password`]).
    ///
    /// # Errors
    ///
    /// [`ConfigError::Invalid`] naming the offending key.
    pub fn validate(&self) -> Result<()> {
        if !(MIN_ARGON2_MEMORY_KIB..=MAX_ARGON2_MEMORY_KIB).contains(&self.argon2_memory_kib) {
            return Err(Error::Config(ConfigError::Invalid {
                key: "argon2_memory_kib".to_string(),
                msg: format!(
                    "must be between {MIN_ARGON2_MEMORY_KIB} and {MAX_ARGON2_MEMORY_KIB}, got {}",
                    self.argon2_memory_kib
                ),
            }));
        }
        if !(1..=MAX_ARGON2_ITERATIONS).contains(&self.argon2_iterations) {
            return Err(Error::Config(ConfigError::Invalid {
                key: "argon2_iterations".to_string(),
                msg: format!(
                    "must be between 1 and {MAX_ARGON2_ITERATIONS}, got {}",
                    self.argon2_iterations
                ),
            }));
        }
        if !(1..=MAX_ARGON2_PARALLELISM).contains(&self.argon2_parallelism) {
            return Err(Error::Config(ConfigError::Invalid {
                key: "argon2_parallelism".to_string(),
                msg: format!(
                    "must be between 1 and {MAX_ARGON2_PARALLELISM}, got {}",
                    self.argon2_parallelism
                ),
            }));
        }
        let minimum_memory = MIN_ARGON2_MEMORY_KIB.saturating_mul(self.argon2_parallelism);
        if self.argon2_memory_kib < minimum_memory {
            return Err(Error::Config(ConfigError::Invalid {
                key: "argon2_memory_kib".to_string(),
                msg: format!(
                    "must be at least 8 KiB per lane ({minimum_memory} for {} lanes), got {}",
                    self.argon2_parallelism, self.argon2_memory_kib
                ),
            }));
        }
        if self.min_length == 0 || self.max_length < self.min_length {
            return Err(Error::Config(ConfigError::Invalid {
                key: "password_length".to_string(),
                msg: format!(
                    "min_length must be >= 1 and <= max_length (got {} and {})",
                    self.min_length, self.max_length
                ),
            }));
        }
        if self.max_length > MAX_VERIFY_PASSWORD_LEN {
            return Err(Error::Config(ConfigError::Invalid {
                key: "max_length".to_string(),
                msg: format!(
                    "must not exceed the verifier's {MAX_VERIFY_PASSWORD_LEN}-character cap, got {}",
                    self.max_length
                ),
            }));
        }
        Ok(())
    }
}

impl From<&ferris_core::AuthConfig> for PasswordConfig {
    fn from(config: &ferris_core::AuthConfig) -> Self {
        Self::from_auth_config(config)
    }
}

/// Validate password against policy
///
/// Length is counted in **characters**, not bytes: a passphrase with accents or
/// CJK text must be judged by how much a human typed, and a byte count would
/// both reject and accept the wrong things.
pub fn validate_password(
    password: &str,
    config: &PasswordConfig,
) -> std::result::Result<(), PasswordError> {
    let length = password.chars().count();
    if length < config.min_length {
        return Err(PasswordError::TooShort(config.min_length));
    }

    if length > config.max_length {
        return Err(PasswordError::TooLong(config.max_length));
    }

    if config.require_uppercase && !password.chars().any(|c| c.is_uppercase()) {
        return Err(PasswordError::MissingUppercase);
    }

    if config.require_lowercase && !password.chars().any(|c| c.is_lowercase()) {
        return Err(PasswordError::MissingLowercase);
    }

    if config.require_numbers && !password.chars().any(|c| c.is_ascii_digit()) {
        return Err(PasswordError::MissingNumber);
    }

    if config.require_symbols && !password.chars().any(|c| c.is_ascii_punctuation()) {
        return Err(PasswordError::MissingSymbol);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_validation() {
        let config = PasswordConfig::default();

        // Valid password
        assert!(validate_password("Password123", &config).is_ok());

        // Too short
        assert!(matches!(
            validate_password("Pass1", &config),
            Err(PasswordError::TooShort(_))
        ));

        // Missing uppercase
        assert!(matches!(
            validate_password("password123", &config),
            Err(PasswordError::MissingUppercase)
        ));

        // Missing lowercase
        assert!(matches!(
            validate_password("PASSWORD123", &config),
            Err(PasswordError::MissingLowercase)
        ));

        // Missing number
        assert!(matches!(
            validate_password("Password", &config),
            Err(PasswordError::MissingNumber)
        ));
    }

    #[test]
    fn password_policy_counts_characters_not_bytes() {
        let config = PasswordConfig {
            require_uppercase: false,
            require_lowercase: false,
            require_numbers: false,
            ..PasswordConfig::default()
        };
        // Eight accented characters are eight characters, not sixteen bytes:
        // they satisfy an eight-character minimum.
        assert!(validate_password("áéíóúñçü", &config).is_ok());
        assert!(matches!(
            validate_password("áéíóúñç", &config),
            Err(PasswordError::TooShort(8))
        ));
    }

    /// The Argon2 knobs and the length policy come from configuration, so a
    /// bad value must fail with a clear error instead of an allocation failure
    /// or a hash that no verification can accept.
    #[test]
    fn password_config_rejects_out_of_range_values() {
        assert!(PasswordConfig::default().validate().is_ok());

        let cases = [
            PasswordConfig {
                argon2_memory_kib: MIN_ARGON2_MEMORY_KIB - 1,
                ..PasswordConfig::default()
            },
            PasswordConfig {
                argon2_memory_kib: MAX_ARGON2_MEMORY_KIB + 1,
                ..PasswordConfig::default()
            },
            PasswordConfig {
                argon2_iterations: 0,
                ..PasswordConfig::default()
            },
            PasswordConfig {
                argon2_iterations: MAX_ARGON2_ITERATIONS + 1,
                ..PasswordConfig::default()
            },
            PasswordConfig {
                argon2_parallelism: 0,
                ..PasswordConfig::default()
            },
            PasswordConfig {
                argon2_parallelism: MAX_ARGON2_PARALLELISM + 1,
                ..PasswordConfig::default()
            },
            PasswordConfig {
                argon2_memory_kib: MIN_ARGON2_MEMORY_KIB,
                argon2_parallelism: 4,
                ..PasswordConfig::default()
            },
            PasswordConfig {
                min_length: 0,
                ..PasswordConfig::default()
            },
            PasswordConfig {
                min_length: 16,
                max_length: 8,
                ..PasswordConfig::default()
            },
            PasswordConfig {
                max_length: MAX_VERIFY_PASSWORD_LEN + 1,
                ..PasswordConfig::default()
            },
        ];
        for config in cases {
            assert!(
                config.validate().is_err(),
                "must reject invalid config: {config:?}"
            );
        }

        // `hash_secret` runs the check, so a bad knob cannot reach Argon2.
        let bad = PasswordConfig {
            argon2_memory_kib: 0,
            ..PasswordConfig::default()
        };
        assert!(super::super::hash_secret("server-generated", &bad).is_err());
    }

    /// The `[auth]` section is what makes the operator's password policy real:
    /// the mapping must move exactly the five policy knobs and leave the Argon2
    /// cost at its defaults.
    #[test]
    fn auth_config_maps_the_password_policy_and_keeps_argon2_defaults() {
        let auth = ferris_core::AuthConfig {
            password_min_length: 14,
            password_require_uppercase: false,
            password_require_lowercase: true,
            password_require_numbers: false,
            password_require_symbols: true,
            ..ferris_core::AuthConfig::default()
        };

        let config = PasswordConfig::from_auth_config(&auth);
        assert_eq!(config.min_length, 14);
        assert!(!config.require_uppercase);
        assert!(config.require_lowercase);
        assert!(!config.require_numbers);
        assert!(config.require_symbols);

        let defaults = PasswordConfig::default();
        assert_eq!(config.max_length, defaults.max_length);
        assert_eq!(config.argon2_memory_kib, defaults.argon2_memory_kib);
        assert_eq!(config.argon2_iterations, defaults.argon2_iterations);
        assert_eq!(config.argon2_parallelism, defaults.argon2_parallelism);

        let from: PasswordConfig = (&auth).into();
        assert_eq!(from.min_length, config.min_length);
        assert_eq!(from.require_symbols, config.require_symbols);
        assert_eq!(from.argon2_memory_kib, config.argon2_memory_kib);
    }
}
