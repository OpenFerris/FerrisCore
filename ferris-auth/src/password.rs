//! Password hashing and verification using Argon2

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use ferris_core::{ConfigError, Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Password configuration
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

/// Password hashing errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordError {
    TooShort(usize),
    TooLong(usize),
    MissingUppercase,
    MissingLowercase,
    MissingNumber,
    MissingSymbol,
    HashingFailed(String),
    VerificationFailed(String),
}

impl fmt::Display for PasswordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PasswordError::TooShort(min) => {
                write!(f, "Password must be at least {} characters", min)
            }
            PasswordError::TooLong(max) => write!(f, "Password must not exceed {} characters", max),
            PasswordError::MissingUppercase => {
                write!(f, "Password must contain at least one uppercase letter")
            }
            PasswordError::MissingLowercase => {
                write!(f, "Password must contain at least one lowercase letter")
            }
            PasswordError::MissingNumber => write!(f, "Password must contain at least one number"),
            PasswordError::MissingSymbol => write!(f, "Password must contain at least one symbol"),
            PasswordError::HashingFailed(e) => write!(f, "Password hashing failed: {}", e),
            PasswordError::VerificationFailed(e) => {
                write!(f, "Password verification failed: {}", e)
            }
        }
    }
}

impl std::error::Error for PasswordError {}

impl From<PasswordError> for Error {
    fn from(e: PasswordError) -> Self {
        Error::Validation(ferris_core::ValidationError::InvalidValue {
            field: "password".to_string(),
            message: e.to_string(),
        })
    }
}

/// Validate password against policy
pub fn validate_password(
    password: &str,
    config: &PasswordConfig,
) -> std::result::Result<(), PasswordError> {
    if password.len() < config.min_length {
        return Err(PasswordError::TooShort(config.min_length));
    }

    if password.len() > config.max_length {
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

/// Hash a password using Argon2
pub fn hash_password(password: &str, config: &PasswordConfig) -> Result<String> {
    // Validate password first
    validate_password(password, config)?;

    hash_secret(password, config)
}

/// Hash a **server-generated secret** with Argon2, skipping the user-facing
/// password policy.
///
/// `validate_password` enforces rules that only make sense for a password a
/// human chose (min length, character classes). Backup codes, API secrets and
/// similar machine-generated credentials are neither — they are already
/// high-entropy, and running the password policy over them would reject valid
/// values. The Argon2 parameters are the caller's, so the stored cost is
/// whatever the deployment configured.
///
/// # Errors
///
/// [`ConfigError::Invalid`] for an unusable Argon2 parameter set, or
/// [`PasswordError::HashingFailed`].
pub fn hash_secret(secret: &str, config: &PasswordConfig) -> Result<String> {
    let argon2 = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(
            config.argon2_memory_kib,
            config.argon2_iterations,
            config.argon2_parallelism,
            None,
        )
        .map_err(|e| {
            Error::Config(ConfigError::Invalid {
                key: "argon2".to_string(),
                msg: e.to_string(),
            })
        })?,
    );

    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2
        .hash_password(secret.as_bytes(), &salt)
        .map_err(|e| PasswordError::HashingFailed(e.to_string()))?;

    Ok(hash.to_string())
}

/// Hash a server-generated secret with the default Argon2 parameters — the
/// counterpart of [`hash_password_default`] for values that must not go through
/// the password policy.
///
/// # Errors
///
/// A hashing failure from the default Argon2 configuration.
pub fn hash_secret_default(secret: &str) -> Result<String> {
    hash_secret(secret, &PasswordConfig::default())
}

/// Hard cap on the input length accepted by [`verify_password`].
///
/// Argon2 allocates and processes memory proportional to the input; without a
/// cap an attacker can force expensive work with huge payloads. Passwords are
/// validated at hash time (`PasswordConfig::max_length`), so anything beyond
/// this bound can never match a legitimately stored hash.
pub const MAX_VERIFY_PASSWORD_LEN: usize = 1024;

/// Verify a password against a hash
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    if password.len() > MAX_VERIFY_PASSWORD_LEN {
        return Ok(false);
    }

    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| PasswordError::VerificationFailed(format!("Invalid password hash: {e}")))?;

    let argon2 = Argon2::default();

    match argon2.verify_password(password.as_bytes(), &parsed_hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(PasswordError::VerificationFailed(e.to_string()).into()),
    }
}

/// Hash a password with default config
pub fn hash_password_default(password: &str) -> Result<String> {
    hash_password(password, &PasswordConfig::default())
}

/// Verify a password with default config
pub fn verify_password_default(password: &str, hash: &str) -> Result<bool> {
    verify_password(password, hash)
}

/// Generate a secure random password
pub fn generate_password(length: usize, config: &PasswordConfig) -> Result<String> {
    use rand::Rng;
    use rand::distributions::{Alphanumeric, DistString};

    let mut rng = rand::thread_rng();
    let mut password = String::new();

    // Ensure we have at least one of each required character type
    if config.require_uppercase {
        password.push(rng.gen_range('A'..='Z'));
    }
    if config.require_lowercase {
        password.push(rng.gen_range('a'..='z'));
    }
    if config.require_numbers {
        password.push(rng.gen_range('0'..='9'));
    }
    if config.require_symbols {
        let symbols = "!@#$%^&*()_+-=[]{}|;:,.<>?";
        password.push(
            symbols
                .chars()
                .nth(rng.gen_range(0..symbols.len()))
                .unwrap(),
        );
    }

    // Fill the rest with alphanumeric
    let remaining = length.saturating_sub(password.len());
    if remaining > 0 {
        password.push_str(&Alphanumeric.sample_string(&mut rng, remaining));
    }

    // Shuffle the password
    let mut chars: Vec<char> = password.chars().collect();
    rand::seq::SliceRandom::shuffle(chars.as_mut_slice(), &mut rng);
    password = chars.into_iter().collect();

    // Validate the generated password
    validate_password(&password, config)?;

    Ok(password)
}

/// Whether a stored hash was produced with different parameters than `config`
/// asks for, i.e. it should be re-hashed on the next successful login.
///
/// This is what makes a cost upgrade real: raising `argon2_memory_kib` (or
/// `argon2_iterations`/`argon2_parallelism`) only strengthens accounts as they
/// log in, so the old hash has to be recognised as stale. Returning `false`
/// unconditionally — as this did — made the upgrade a silent no-op.
///
/// A hash that cannot be parsed is reported as needing a rehash: the caller
/// can then replace the unreadable value with a fresh, known-good one.
///
/// # Errors
///
/// Returns [`PasswordError::VerificationFailed`] when `hash` is not a valid
/// PHC password hash.
pub fn needs_rehash(hash: &str, config: &PasswordConfig) -> Result<bool> {
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| PasswordError::VerificationFailed(format!("Invalid password hash: {e}")))?;

    // A different algorithm is always worth re-hashing to the current one.
    if parsed_hash.algorithm.as_str() != "argon2id" {
        return Ok(true);
    }

    // A different (older) Argon2 version is worth re-hashing too.
    if parsed_hash.version != Some(Version::V0x13.into()) {
        return Ok(true);
    }

    // Compare the stored cost parameters against the configured ones. A hash
    // written before a parameter bump carries the old `m`/`t`/`p` and must be
    // re-derived with the new ones.
    let stored = Params::try_from(&parsed_hash).map_err(|e| {
        PasswordError::VerificationFailed(format!("Invalid Argon2 parameters: {e}"))
    })?;
    Ok(stored.m_cost() != config.argon2_memory_kib
        || stored.t_cost() != config.argon2_iterations
        || stored.p_cost() != config.argon2_parallelism)
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
    fn test_hash_and_verify() {
        let config = PasswordConfig::default();
        let password = "SecurePass123!";

        let hash = hash_password(password, &config).unwrap();
        assert!(!hash.is_empty());

        let valid = verify_password(password, &hash).unwrap();
        assert!(valid);

        let invalid = verify_password("WrongPass123!", &hash).unwrap();
        assert!(!invalid);
    }

    #[test]
    fn test_generate_password() {
        let config = PasswordConfig::default();
        let password = generate_password(16, &config).unwrap();

        assert_eq!(password.len(), 16);
        assert!(validate_password(&password, &config).is_ok());
    }

    #[test]
    fn test_needs_rehash() {
        let config = PasswordConfig::default();
        let password = "TestPass123!";
        let hash = hash_password(password, &config).unwrap();

        let needs = needs_rehash(&hash, &config).unwrap();
        assert!(!needs); // Should not need rehash with same config
    }

    /// AUTH3: raising the Argon2 cost must mark existing hashes as stale, so
    /// the next login re-derives them under the stronger parameters. The old
    /// implementation returned `false` unconditionally, making the upgrade a
    /// silent no-op.
    #[test]
    fn a_cost_upgrade_marks_the_stored_hash_stale() {
        let password = "TestPass123!";
        let weak = PasswordConfig {
            argon2_memory_kib: 8 * 1024,
            argon2_iterations: 1,
            argon2_parallelism: 1,
            ..PasswordConfig::default()
        };
        let hash = hash_password(password, &weak).unwrap();
        assert!(!needs_rehash(&hash, &weak).unwrap(), "same params: current");

        // Every cost dimension, raised on its own, forces a rehash.
        let mut memory = weak.clone();
        memory.argon2_memory_kib *= 2;
        let mut iterations = weak.clone();
        iterations.argon2_iterations += 1;
        let mut parallelism = weak.clone();
        parallelism.argon2_parallelism += 1;
        for stronger in [&memory, &iterations, &parallelism] {
            assert!(
                needs_rehash(&hash, stronger).unwrap(),
                "raising a cost must mark the hash stale: {stronger:?}"
            );
        }

        // A hash that cannot be parsed is reported as needing a rehash rather
        // than failing the login path.
        assert!(needs_rehash("not-a-phc-hash", &weak).is_err());
    }
}
