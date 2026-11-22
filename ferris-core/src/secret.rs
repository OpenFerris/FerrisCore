//! Recoverable secrets at rest: authenticated encryption for `Secret` fields.
//!
//! [`crate::password`] is deliberately one-way (Argon2): a login verifies, it
//! never recovers. An OAuth client secret, a webhook signing key or an SMTP
//! password is the opposite — the framework has to *hand the plaintext back* to
//! the connector that signs or authenticates with it. Hashing those values
//! (which is what declaring them `Password` did) made every integration read
//! `"********"` and silently fail.
//!
//! This module is the recoverable half:
//!
//! * **AES-256-GCM** from `ring`, never a hand-rolled construction.
//! * One **random 96-bit nonce per value**, so two saves of the same secret
//!   produce different ciphertexts.
//! * A **versioned envelope** — `ferris:v1:<key-id>:<base64url payload>` — with
//!   the key id derived from the key material, so a key can be rotated without
//!   re-encrypting every row up front.
//! * The `DocType` and fieldname are bound as **associated data**, so a
//!   ciphertext copied into another field (or another `DocType`'s blob) fails
//!   authentication instead of decrypting into the wrong slot.
//!
//! # Trust model
//!
//! Installing a keyring is a boot action; the secret is only decrypted by
//! [`decrypt_secret`], which the document engine exposes exclusively through
//! `DocumentEngine::read_secret*` to an [`EngineContext`] that opted in with
//! `reveal_secrets()`. The plaintext is wrapped in [`SecretValue`], whose
//! `Debug` is redacted and which has no `Display`/`Serialize`, so a document
//! read — or a `tracing::debug!(?doc)` — can never print it.
//!
//! # Rotation and failures
//!
//! A keyring carries the primary key (used to encrypt) plus `previous_keys`
//! (accepted when decrypting). A value whose key id is no longer installed
//! returns [`SecretError::UnknownKeyId`]; one that is not a v1 envelope (an
//! Argon2 hash written while the field was `Password`, or a value from a future
//! format) returns [`SecretError::NotRecoverable`]. Neither is ever silently
//! turned into an empty string.
//!
//! [`EngineContext`]: # "document engine context, see ferris-document"

#![allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.

use crate::config::SecretsConfig;
use crate::{ConfigError, Error, Result, SecretError};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom, SystemRandom};
use std::fmt;
use std::sync::OnceLock;

/// What a `Secret` field reads back as for every ordinary caller.
///
/// The same literal as [`crate::password::PASSWORD_MASK`] on purpose: the Desk
/// renders one masked-input state, and "the mask means unchanged" is one rule.
/// Kept as its own constant so the two field types can diverge later without a
/// silent behaviour change in the other.
pub const SECRET_MASK: &str = crate::password::PASSWORD_MASK;

/// Envelope scheme (before the version): `ferris:v1:<kid>:<payload>`.
const ENVELOPE_SCHEME: &str = "ferris";
/// Envelope version. A future format bumps this; an unknown one is
/// [`SecretError::NotRecoverable`] rather than a decryption attempt.
const ENVELOPE_VERSION: &str = "v1";
/// AES-GCM key length, bytes.
const KEY_LEN: usize = 32;
/// AES-GCM nonce length, bytes.
const NONCE_LEN: usize = 12;
/// AES-GCM authentication tag length, bytes.
const TAG_LEN: usize = 16;
/// Hex characters of the SHA-256 key fingerprint kept as the key id.
const KEY_ID_LEN: usize = 8;

/// A 32-byte AES-256-GCM key.
///
/// `Debug` is redacted and there is no `Display`/`Serialize`: the key material
/// only leaves through the AEAD calls below.
pub struct SecretKey {
    bytes: [u8; KEY_LEN],
    /// First [`KEY_ID_LEN`] hex characters of `SHA-256(bytes)`.
    id: String,
}

impl SecretKey {
    /// Parse a key from configuration: 64 hex characters or base64 (URL-safe or
    /// standard, with or without padding), always exactly 32 bytes.
    ///
    /// # Errors
    ///
    /// [`SecretError::KeyInvalid`] when the material is not 32 bytes or is not
    /// valid hex/base64.
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(Error::Secret(SecretError::KeyInvalid(
                "the key is empty".to_string(),
            )));
        }
        let bytes = hex_decode_32(raw)
            .or_else(|| URL_SAFE_NO_PAD.decode(raw).ok().and_then(fixed_32))
            .or_else(|| {
                base64::engine::general_purpose::STANDARD
                    .decode(raw)
                    .ok()
                    .and_then(fixed_32)
            })
            .ok_or_else(|| {
                Error::Secret(SecretError::KeyInvalid(
                    "expected 32 bytes as 64 hex characters or base64".to_string(),
                ))
            })?;
        Ok(Self::from_bytes(bytes))
    }

    fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        let fingerprint = digest(&SHA256, &bytes);
        let id = hex_encode(&fingerprint.as_ref()[..KEY_ID_LEN / 2]);
        Self { bytes, id }
    }

    /// The key id that travels in the envelope.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    fn aead(&self) -> Result<LessSafeKey> {
        let unbound = UnboundKey::new(&AES_256_GCM, &self.bytes).map_err(|_| {
            Error::Secret(SecretError::KeyInvalid(
                "AES-256-GCM rejected the 32-byte key".to_string(),
            ))
        })?;
        Ok(LessSafeKey::new(unbound))
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretKey")
            .field("id", &self.id)
            .field("material", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// The primary key plus the retired keys still accepted for decryption.
pub struct SecretKeyring {
    primary: SecretKey,
    previous: Vec<SecretKey>,
}

impl SecretKeyring {
    /// A ring with one encryption key and any number of retired keys.
    #[must_use]
    pub const fn new(primary: SecretKey, previous: Vec<SecretKey>) -> Self {
        Self { primary, previous }
    }

    fn find(&self, id: &str) -> Option<&SecretKey> {
        if self.primary.id == id {
            return Some(&self.primary);
        }
        self.previous.iter().find(|key| key.id == id)
    }

    /// Whether two rings hold the same material, for the "re-installing is
    /// idempotent, replacing is refused" rule.
    fn same_material(&self, other: &Self) -> bool {
        let mut mine: Vec<&str> = self.all_ids();
        let mut theirs: Vec<&str> = other.all_ids();
        mine.sort_unstable();
        theirs.sort_unstable();
        mine == theirs
    }

    fn all_ids(&self) -> Vec<&str> {
        std::iter::once(self.primary.id())
            .chain(self.previous.iter().map(SecretKey::id))
            .collect()
    }
}

impl fmt::Debug for SecretKeyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretKeyring")
            .field("primary", &self.primary)
            .field(
                "previous_ids",
                &self.previous.iter().map(SecretKey::id).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Which logical slot a ciphertext belongs to, bound as AEAD associated data.
///
/// Copying a ciphertext from `Webhook.webhook_secret` into, say,
/// `OAuth Client.client_secret` therefore fails to authenticate even though the
/// key is the same: the tag covers the location.
#[derive(Debug, Clone, Copy)]
pub struct SecretScope<'a> {
    /// `DocType` whose document holds the value (the child `DocType` for a child
    /// table field).
    pub doctype: &'a str,
    /// The field's `fieldname`.
    pub field: &'a str,
}

impl SecretScope<'_> {
    fn aad(&self) -> Vec<u8> {
        let mut aad = Vec::with_capacity(self.doctype.len() + self.field.len() + 1);
        aad.extend_from_slice(self.doctype.as_bytes());
        aad.push(0);
        aad.extend_from_slice(self.field.as_bytes());
        aad
    }
}

/// A decrypted secret.
///
/// The only way out is explicit — [`Self::expose`] or [`Self::into_inner`] — and
/// `Debug` prints `"<redacted>"`. There is deliberately no `Display`, no
/// `Serialize` and no `Clone`, so a secret cannot be logged or persisted by
/// accident.
pub struct SecretValue(String);

impl SecretValue {
    /// Wrap a plaintext.
    ///
    /// Public so a caller that legitimately holds a secret (a test fixture, a
    /// plugin bridging its own store) can carry it through the same redacted
    /// type instead of inventing a second one. The value is not validated: the
    /// engine's encrypted reader only ever produces non-empty plaintext.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the plaintext.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Take the plaintext, for a caller that is about to hand it to a transport
    /// (an HTTP form field, an SMTP credential).
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }

    /// Whether the value is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"<redacted>\"")
    }
}

static KEYRING: OnceLock<SecretKeyring> = OnceLock::new();

/// Install the process-wide keyring (called once at boot).
///
/// The first call wins. Re-installing the **same** material is a no-op that
/// returns `true`, so a re-run of the boot sequence cannot fail; a **different**
/// ring is refused, because silently swapping the key would leave every stored
/// ciphertext undecryptable while new writes kept succeeding.
#[must_use]
pub fn install_keyring(keyring: SecretKeyring) -> bool {
    install(&KEYRING, keyring)
}

#[must_use]
fn install(slot: &OnceLock<SecretKeyring>, keyring: SecretKeyring) -> bool {
    match slot.get() {
        Some(installed) => installed.same_material(&keyring),
        None => slot.set(keyring).is_ok(),
    }
}

/// Whether the process has a keyring installed.
#[must_use]
pub fn is_keyring_installed() -> bool {
    KEYRING.get().is_some()
}

/// Build a keyring from the `[secrets]` configuration.
///
/// # Errors
///
/// [`ConfigError::Missing`] when `previous_keys` are configured without a
/// primary key, and [`SecretError::KeyInvalid`] for unusable material.
pub fn keyring_from_config(config: &SecretsConfig) -> Result<Option<SecretKeyring>> {
    let primary = config
        .key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let Some(primary) = primary else {
        if config
            .previous_keys
            .iter()
            .any(|key| !key.trim().is_empty())
        {
            return Err(Error::Config(ConfigError::Missing(
                "secrets.key (previous_keys are configured without a primary key)".to_string(),
            )));
        }
        return Ok(None);
    };
    let mut previous = Vec::with_capacity(config.previous_keys.len());
    for key in &config.previous_keys {
        if key.trim().is_empty() {
            continue;
        }
        previous.push(SecretKey::parse(key)?);
    }
    Ok(Some(SecretKeyring::new(
        SecretKey::parse(primary)?,
        previous,
    )))
}

/// Install the keyring described by `[secrets]`, if one is configured.
///
/// Returns `true` when a ring is installed (now or by an earlier call).
///
/// # Errors
///
/// Whatever [`keyring_from_config`] returns.
pub fn install_keyring_from_config(config: &SecretsConfig) -> Result<bool> {
    keyring_from_config(config)?.map_or(Ok(false), |keyring| Ok(install_keyring(keyring)))
}

/// Whether `value` has the shape of a v1 envelope.
///
/// Used to tell "encrypted" from "an Argon2 hash written before the field was
/// migrated" without attempting a decryption (and without a key at all).
#[must_use]
pub fn is_secret_envelope(value: &str) -> bool {
    let Some(rest) = value.strip_prefix(ENVELOPE_SCHEME) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(':') else {
        return false;
    };
    let mut parts = rest.splitn(3, ':');
    matches!(parts.next(), Some(version) if version == ENVELOPE_VERSION)
        && parts.next().is_some_and(|key_id| !key_id.is_empty())
        && parts.next().is_some_and(|payload| !payload.is_empty())
}

/// Encrypt `plaintext` for `scope` under the installed keyring.
///
/// # Errors
///
/// [`SecretError::KeyMissing`] when no keyring was installed (fails closed: a
/// secret is never written in clear), or [`SecretError::KeyInvalid`] when the
/// AEAD rejects the key material.
pub fn encrypt_secret(scope: SecretScope<'_>, plaintext: &str) -> Result<String> {
    encrypt_with(KEYRING.get(), scope, plaintext)
}

fn encrypt_with(
    keyring: Option<&SecretKeyring>,
    scope: SecretScope<'_>,
    plaintext: &str,
) -> Result<String> {
    let keyring = keyring.ok_or_else(|| Error::Secret(SecretError::KeyMissing))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    SystemRandom::new().fill(&mut nonce_bytes).map_err(|_| {
        Error::Secret(SecretError::KeyInvalid(
            "the system random source failed to produce a nonce".to_string(),
        ))
    })?;
    let mut payload = plaintext.as_bytes().to_vec();
    keyring
        .primary
        .aead()?
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(scope.aad()),
            &mut payload,
        )
        .map_err(|_| {
            Error::Secret(SecretError::KeyInvalid(
                "AES-256-GCM failed to seal the secret".to_string(),
            ))
        })?;
    let mut encoded = Vec::with_capacity(NONCE_LEN + payload.len());
    encoded.extend_from_slice(&nonce_bytes);
    encoded.extend_from_slice(&payload);
    Ok(format!(
        "{ENVELOPE_SCHEME}:{ENVELOPE_VERSION}:{}:{}",
        keyring.primary.id,
        URL_SAFE_NO_PAD.encode(encoded)
    ))
}

/// Decrypt a v1 envelope for `scope` under the installed keyring.
///
/// # Errors
///
/// * [`SecretError::KeyMissing`] — no keyring installed.
/// * [`SecretError::NotRecoverable`] — not a v1 envelope (an Argon2 hash from
///   the old `Password` declaration, a future format, or corrupt base64).
/// * [`SecretError::UnknownKeyId`] — encrypted with a key that is not in
///   `previous_keys` any more.
/// * [`SecretError::AuthenticationFailed`] — the tag does not verify (wrong
///   key, value moved to another field/DocType, or tampering).
pub fn decrypt_secret(scope: SecretScope<'_>, envelope: &str) -> Result<SecretValue> {
    decrypt_with(KEYRING.get(), scope, envelope)
}

fn decrypt_with(
    keyring: Option<&SecretKeyring>,
    scope: SecretScope<'_>,
    envelope: &str,
) -> Result<SecretValue> {
    let unrecoverable = || {
        Error::Secret(SecretError::NotRecoverable {
            doctype: scope.doctype.to_string(),
            field: scope.field.to_string(),
        })
    };
    let keyring = keyring.ok_or_else(|| Error::Secret(SecretError::KeyMissing))?;
    let rest = envelope
        .strip_prefix(ENVELOPE_SCHEME)
        .and_then(|rest| rest.strip_prefix(':'))
        .ok_or_else(unrecoverable)?;
    let mut parts = rest.splitn(3, ':');
    let version = parts.next().ok_or_else(unrecoverable)?;
    if version != ENVELOPE_VERSION {
        return Err(unrecoverable());
    }
    let key_id = parts.next().ok_or_else(unrecoverable)?;
    let payload = parts.next().ok_or_else(unrecoverable)?;
    let key = keyring.find(key_id).ok_or_else(|| {
        Error::Secret(SecretError::UnknownKeyId {
            doctype: scope.doctype.to_string(),
            field: scope.field.to_string(),
            key_id: key_id.to_string(),
        })
    })?;
    let mut encoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| unrecoverable())?;
    if encoded.len() < NONCE_LEN + TAG_LEN {
        return Err(unrecoverable());
    }
    let nonce_bytes: [u8; NONCE_LEN] = encoded
        .get(..NONCE_LEN)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(unrecoverable)?;
    let mut ciphertext = encoded.split_off(NONCE_LEN);
    let plain = key
        .aead()?
        .open_in_place(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(scope.aad()),
            &mut ciphertext,
        )
        .map_err(|_| {
            Error::Secret(SecretError::AuthenticationFailed {
                doctype: scope.doctype.to_string(),
                field: scope.field.to_string(),
            })
        })?;
    let text = std::str::from_utf8(plain).map_err(|_| unrecoverable())?;
    Ok(SecretValue::new(text.to_string()))
}

fn fixed_32(bytes: Vec<u8>) -> Option<[u8; KEY_LEN]> {
    bytes.try_into().ok()
}

/// Decode 64 hex characters into 32 bytes; `None` for anything else.
fn hex_decode_32(raw: &str) -> Option<[u8; KEY_LEN]> {
    if raw.len() != KEY_LEN * 2 {
        return None;
    }
    let mut out = [0u8; KEY_LEN];
    for (index, chunk) in raw.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let high = hex_nibble(chunk[0])?;
        let low = hex_nibble(chunk[1])?;
        out[index] = (high << 4) | low;
    }
    Some(out)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

const HEX: [u8; 16] = *b"0123456789abcdef";

#[cfg(test)]
mod tests;
