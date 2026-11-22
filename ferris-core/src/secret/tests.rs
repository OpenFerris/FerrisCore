use super::*;

fn key(byte: u8) -> SecretKey {
    SecretKey::from_bytes([byte; KEY_LEN])
}

fn keyring() -> SecretKeyring {
    SecretKeyring::new(key(7), Vec::new())
}

fn scope() -> SecretScope<'static> {
    SecretScope {
        doctype: "Webhook",
        field: "webhook_secret",
    }
}

#[test]
fn a_secret_round_trips_and_the_envelope_does_not_carry_it() {
    let ring = keyring();
    let envelope = encrypt_with(Some(&ring), scope(), "s3cret").expect("encrypts");
    assert!(envelope.starts_with("ferris:v1:"), "{envelope}");
    assert!(
        !envelope.contains("s3cret"),
        "the plaintext leaked: {envelope}"
    );
    assert!(is_secret_envelope(&envelope));

    let value = decrypt_with(Some(&ring), scope(), &envelope).expect("decrypts");
    assert_eq!(value.expose(), "s3cret");
    assert!(!value.is_empty());
}

#[test]
fn two_encryptions_of_the_same_value_differ() {
    let ring = keyring();
    let first = encrypt_with(Some(&ring), scope(), "same").expect("encrypts");
    let second = encrypt_with(Some(&ring), scope(), "same").expect("encrypts");
    assert_ne!(first, second, "the nonce must be fresh per value");
    assert_eq!(
        decrypt_with(Some(&ring), scope(), &first)
            .expect("decrypts")
            .expose(),
        "same"
    );
    assert_eq!(
        decrypt_with(Some(&ring), scope(), &second)
            .expect("decrypts")
            .expose(),
        "same"
    );
}

#[test]
fn a_ciphertext_moved_to_another_field_or_doctype_fails_authentication() {
    let ring = keyring();
    let envelope = encrypt_with(Some(&ring), scope(), "s3cret").expect("encrypts");

    for other in [
        SecretScope {
            doctype: "OAuth Client",
            field: "webhook_secret",
        },
        SecretScope {
            doctype: "Webhook",
            field: "client_secret",
        },
    ] {
        let error = decrypt_with(Some(&ring), other, &envelope).expect_err("must not decrypt");
        assert!(
            matches!(
                error,
                Error::Secret(SecretError::AuthenticationFailed { .. })
            ),
            "{error:?}"
        );
    }
}

#[test]
fn a_tampered_ciphertext_fails_authentication() {
    let ring = keyring();
    let envelope = encrypt_with(Some(&ring), scope(), "s3cret").expect("encrypts");
    let mut bytes = envelope.into_bytes();
    // Flip one character of the base64 payload, not of the prefix.
    let last = bytes.len() - 1;
    bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(bytes).expect("utf8");

    let error = decrypt_with(Some(&ring), scope(), &tampered).expect_err("must not decrypt");
    assert!(
        matches!(
            error,
            Error::Secret(
                SecretError::AuthenticationFailed { .. } | SecretError::NotRecoverable { .. }
            )
        ),
        "{error:?}"
    );
}

#[test]
fn without_a_keyring_encrypt_and_decrypt_fail_closed() {
    // The process-wide hook cannot be reset, so the no-key branch is asserted
    // over an explicit slot — the same shape `password::hash_with` uses.
    let error = encrypt_with(None, scope(), "s3cret").expect_err("no key: refuse to write");
    assert!(matches!(error, Error::Secret(SecretError::KeyMissing)));

    let error = decrypt_with(None, scope(), "ferris:v1:00000000:AAAA").expect_err("no key");
    assert!(matches!(error, Error::Secret(SecretError::KeyMissing)));
}

#[test]
fn a_key_is_exactly_32_bytes_of_hex_or_base64() {
    let hex = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    let parsed = SecretKey::parse(hex).expect("64 hex characters");
    assert_eq!(parsed.id().len(), KEY_ID_LEN);

    let b64 = URL_SAFE_NO_PAD.encode([7u8; KEY_LEN]);
    assert!(SecretKey::parse(&b64).is_ok());
    assert_eq!(
        SecretKey::parse(&b64).expect("url-safe base64").id(),
        SecretKey::parse(&hex_32()).expect("hex").id()
    );

    for bad in ["", "  ", "tooshort", &"00".repeat(31)] {
        let error = SecretKey::parse(bad).expect_err("rejects an invalid length");
        assert!(
            matches!(error, Error::Secret(SecretError::KeyInvalid(_))),
            "{error:?}"
        );
    }
}

fn hex_32() -> String {
    let value = [7u8; KEY_LEN];
    hex_encode(&value)
}

#[test]
fn rotation_decrypts_with_previous_keys_and_rejects_removed_ones() {
    let old = key(1);
    let old_envelope = encrypt_with(
        Some(&SecretKeyring::new(key(1), Vec::new())),
        scope(),
        "s3cret",
    )
    .expect("encrypts with the old key");

    let rotated = SecretKeyring::new(key(2), vec![key(1)]);
    assert_eq!(
        decrypt_with(Some(&rotated), scope(), &old_envelope)
            .expect("the retired key still decrypts")
            .expose(),
        "s3cret"
    );
    // A new write uses the primary key.
    let fresh = encrypt_with(Some(&rotated), scope(), "other").expect("encrypts");
    assert!(
        fresh.starts_with(&format!("ferris:v1:{}:", key(2).id())),
        "{fresh}"
    );

    // Dropping the retired key makes the old ciphertext explicitly unrecoverable.
    let only_new = SecretKeyring::new(key(2), Vec::new());
    let error = decrypt_with(Some(&only_new), scope(), &old_envelope).expect_err("must fail");
    match error {
        Error::Secret(SecretError::UnknownKeyId { key_id, .. }) => {
            assert_eq!(key_id, old.id());
        }
        other => panic!("expected UnknownKeyId, got {other:?}"),
    }
}

#[test]
fn a_legacy_or_unknown_format_is_not_recoverable() {
    let ring = keyring();
    let key_id = key(7).id().to_string();

    // Not an envelope at all: the Argon2 hash a `Password` field wrote before
    // the migration, the mask, and plain text.
    for legacy in [
        "$argon2id$v=19$m=65536,t=3,p=4$c2FsdA$aGFzaA",
        "********",
        "plain text",
    ] {
        let error = decrypt_with(Some(&ring), scope(), legacy).expect_err("must fail");
        assert!(
            matches!(error, Error::Secret(SecretError::NotRecoverable { .. })),
            "{legacy}: {error:?}"
        );
        assert!(!is_secret_envelope(legacy), "{legacy}");
    }

    // An envelope-shaped value with an unknown version, corrupt payload, or a
    // payload shorter than nonce+tag is `NotRecoverable` too.
    for broken in [
        "ferris:v2:00000000:AAAA".to_string(),
        format!("ferris:v1:{key_id}:not-base64!!"),
        format!("ferris:v1:{key_id}:AAAA"),
    ] {
        let error = decrypt_with(Some(&ring), scope(), &broken).expect_err("must fail");
        assert!(
            matches!(error, Error::Secret(SecretError::NotRecoverable { .. })),
            "{broken}: {error:?}"
        );
    }

    // A structurally valid envelope whose key id is not installed is named as
    // such: the operator has to restore the retired key, not re-enter the value.
    let error =
        decrypt_with(Some(&ring), scope(), "ferris:v1:00000000:AAAA").expect_err("unknown key");
    assert!(
        matches!(error, Error::Secret(SecretError::UnknownKeyId { .. })),
        "{error:?}"
    );
}

#[test]
fn secret_value_debug_is_redacted() {
    let value = SecretValue::new("hunter2".to_string());
    assert_eq!(format!("{value:?}"), "\"<redacted>\"");
    assert!(!format!("{value:?}").contains("hunter2"));
    assert_eq!(value.expose(), "hunter2");
    assert_eq!(value.into_inner(), "hunter2");
}

#[test]
fn installing_is_idempotent_and_a_different_keyring_is_refused() {
    let slot: OnceLock<SecretKeyring> = OnceLock::new();
    assert!(install(&slot, keyring()));
    assert!(
        install(&slot, keyring()),
        "the same material installs twice"
    );
    assert!(
        !install(&slot, SecretKeyring::new(key(9), Vec::new())),
        "a different primary key is refused"
    );
    assert!(
        !install(&slot, SecretKeyring::new(key(7), vec![key(8)])),
        "adding a previous key changes the material and is refused"
    );
}

#[test]
fn the_config_requires_a_primary_key_when_previous_keys_are_set() {
    let empty = SecretsConfig::default();
    assert!(
        keyring_from_config(&empty)
            .expect("an unconfigured ring is not an error")
            .is_none()
    );

    let previous_only = SecretsConfig {
        key: None,
        previous_keys: vec![hex_32()],
    };
    let error = keyring_from_config(&previous_only).expect_err("must fail loudly");
    assert!(
        matches!(error, Error::Config(ConfigError::Missing(ref what)) if what.contains("secrets.key")),
        "{error:?}"
    );

    let configured = SecretsConfig {
        key: Some(hex_32()),
        previous_keys: vec![hex_encode(&[1u8; KEY_LEN])],
    };
    let ring = keyring_from_config(&configured)
        .expect("valid config")
        .expect("a ring");
    assert_eq!(ring.primary.id(), key(7).id());
}
