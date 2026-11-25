//! Tests for [`super::ApiKeyManager`].
//!
//! The manager has two storage paths — memory and the document engine — so the
//! security-relevant cases run against both: a fix that only holds in one of
//! them is exactly the kind of drift this module exists to catch.

use super::*;

async fn db() -> Db {
    ferris_db::memory_db().await.unwrap()
}

/// A manager over memory and one over a fresh database, so a test can assert
/// the same behaviour on both paths.
async fn managers() -> [ApiKeyManager; 2] {
    let memory = ApiKeyManager::new();
    memory.init().await.unwrap();
    let database = ApiKeyManager::with_db(db().await);
    database.init().await.unwrap();
    [memory, database]
}

#[tokio::test]
async fn test_api_key_lifecycle() {
    let manager = ApiKeyManager::with_db(db().await);
    manager.init().await.unwrap();

    let (record, secret) = manager
        .create("user@example.com", Some("integration".to_string()), None)
        .await
        .unwrap();
    assert!(!secret.is_empty());
    assert!(!record.key.is_empty());

    // Correct credentials, both directly and via the Frappe `token` scheme.
    assert!(
        manager
            .verify(&record.key, &secret)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        manager
            .authenticate(&format!("{}:{}", record.key, secret))
            .await
            .unwrap()
            .is_some()
    );

    // Wrong secret / unknown key.
    assert!(manager.verify(&record.key, "nope").await.unwrap().is_none());
    assert!(manager.verify("unknown", &secret).await.unwrap().is_none());

    // Listing hides nothing but the secret hash is skipped on serialize.
    let listed = manager.list("user@example.com").await.unwrap();
    assert_eq!(listed.len(), 1);
    let json = serde_json::to_string(&listed[0]).unwrap();
    assert!(!json.contains(&secret));

    // Revoked keys no longer verify.
    manager.revoke(&record.id).await.unwrap();
    assert!(
        manager
            .verify(&record.key, &secret)
            .await
            .unwrap()
            .is_none()
    );
}

/// A use inside the window must not write: the hot path stays a read, and
/// the timestamp returned is the stored one (not a fresher value the row
/// does not have).
#[tokio::test]
async fn last_used_at_is_not_rewritten_on_every_request() {
    let manager = ApiKeyManager::new();
    let (record, secret) = manager
        .create("user@example.com", None, None)
        .await
        .unwrap();

    let first = manager.verify(&record.key, &secret).await.unwrap().unwrap();
    let first_used = first.last_used_at.expect("the first use stamps the key");
    let second = manager.verify(&record.key, &secret).await.unwrap().unwrap();
    assert_eq!(
        second.last_used_at,
        Some(first_used),
        "a second use inside the resolution window must not move the stamp"
    );
}

/// Once the stamp is stale the next successful use refreshes it.
#[tokio::test]
async fn a_stale_last_used_at_is_refreshed() {
    let manager = ApiKeyManager::new();
    let (record, secret) = manager
        .create("user@example.com", None, None)
        .await
        .unwrap();
    let stale = Utc::now() - chrono::Duration::seconds(LAST_USED_RESOLUTION_SECS + 1);
    manager
        .memory
        .write()
        .iter_mut()
        .find(|key| key.id == record.id)
        .expect("the created key")
        .last_used_at = Some(stale);

    let verified = manager.verify(&record.key, &secret).await.unwrap().unwrap();
    let refreshed = verified.last_used_at.expect("refreshed");
    assert!(
        refreshed > stale,
        "a stale stamp must be refreshed: {stale} -> {refreshed}"
    );
    let stored = manager
        .list("user@example.com")
        .await
        .unwrap()
        .into_iter()
        .find(|key| key.id == record.id)
        .expect("stored key");
    assert_eq!(stored.last_used_at, Some(refreshed));
}

#[tokio::test]
async fn test_expired_key_is_invalid() {
    let manager = ApiKeyManager::new();
    let (record, secret) = manager
        .create(
            "user@example.com",
            None,
            Some(Utc::now() - chrono::Duration::seconds(1)),
        )
        .await
        .unwrap();
    assert!(
        manager
            .verify(&record.key, &secret)
            .await
            .unwrap()
            .is_none()
    );
}

/// The row stores `sha256:<64 lowercase hex>` over the secret the caller
/// received, never the secret itself.
#[tokio::test]
async fn the_stored_secret_is_a_sha256_digest() {
    for manager in managers().await {
        let (record, secret) = manager
            .create("user@example.com", None, None)
            .await
            .unwrap();
        let expected = format!("sha256:{}", sha256_hex(secret.as_bytes()));
        assert_eq!(record.secret_hash, expected);
        assert_eq!(record.secret_hash.len(), "sha256:".len() + 64);
        assert!(
            record
                .secret_hash
                .strip_prefix("sha256:")
                .expect("the stored form has the digest prefix")
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        );
        assert!(!record.secret_hash.contains(&secret));
        let listed = manager.list("user@example.com").await.unwrap();
        assert_eq!(listed[0].secret_hash, expected);
    }
}

/// A row written while `secret_hash` was a `Password` field holds an Argon2
/// PHC hash. It must still authenticate, in memory and in the database, and the
/// successful use rewrites it with the digest.
#[tokio::test]
async fn a_legacy_argon2_hash_is_accepted_and_upgraded() {
    for manager in managers().await {
        let (record, secret) = manager
            .create("user@example.com", None, None)
            .await
            .unwrap();
        let legacy = crate::password::hash_secret_default(&secret).unwrap();
        assert!(legacy.starts_with("$argon2"));
        manager
            .store_secret_hash(&record.id, &legacy)
            .await
            .unwrap();

        // The wrong secret never matches a legacy row and must not upgrade it.
        assert!(
            manager
                .verify(&record.key, "wrong")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            manager
                .list("user@example.com")
                .await
                .unwrap()
                .remove(0)
                .secret_hash,
            legacy,
            "a failed verification must leave the stored hash alone"
        );

        let verified = manager.verify(&record.key, &secret).await.unwrap();
        let verified = verified.expect("the legacy hash must still authenticate");
        assert_eq!(
            verified.secret_hash,
            hash_secret_for_storage(&secret),
            "a successful legacy verification upgrades the row"
        );
        assert_eq!(
            manager.list("user@example.com").await.unwrap()[0].secret_hash,
            verified.secret_hash,
            "the upgrade is persisted"
        );
    }
}

/// Anything that is not a well-formed digest or a legacy PHC hash is a
/// mismatch. It must neither authenticate nor panic.
#[tokio::test]
async fn a_malformed_stored_secret_fails_closed() {
    let manager = ApiKeyManager::new();
    let (record, secret) = manager
        .create("user@example.com", None, None)
        .await
        .unwrap();
    for malformed in [
        String::new(),
        "plaintext".to_string(),
        "sha256:".to_string(),
        // Right length, not hex.
        format!("sha256:{}", "z".repeat(64)),
        // Hex, wrong length.
        "sha256:abcd".to_string(),
        // Uppercase hex is not the stored form.
        format!("sha256:{}", sha256_hex(&[0u8; 32]).to_ascii_uppercase()),
        // A prefix that looks like Argon2 but parses to nothing.
        "$argon2id$not-a-phc-hash".to_string(),
    ] {
        manager
            .store_secret_hash(&record.id, &malformed)
            .await
            .unwrap();
        assert!(
            manager
                .verify(&record.key, &secret)
                .await
                .unwrap()
                .is_none(),
            "a malformed stored value must fail closed: {malformed:?}"
        );
    }
}

/// The derived `Debug` used to print the hash; the manual one must not.
#[tokio::test]
async fn debug_and_serialization_redact_the_secret_hash() {
    let manager = ApiKeyManager::new();
    let (record, secret) = manager
        .create("user@example.com", None, None)
        .await
        .unwrap();
    let debug = format!("{record:?}");
    assert!(debug.contains("secret_hash"));
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains(&record.secret_hash));
    assert!(!debug.contains(&secret));

    let json = serde_json::to_string(&record).unwrap();
    assert!(!json.contains("secret_hash"));
    assert!(!json.contains(&record.secret_hash));
}

/// A known public key must not allow unlimited guesses at the secret: after the
/// failure budget the key is refused even with the right secret, until the
/// window elapses.
#[tokio::test]
async fn repeated_wrong_secrets_block_the_key() {
    let manager = ApiKeyManager::with_verification_limit(3, 60);
    let (record, secret) = manager
        .create("user@example.com", None, None)
        .await
        .unwrap();

    for _ in 0..3 {
        assert!(
            manager
                .verify(&record.key, "wrong")
                .await
                .unwrap()
                .is_none(),
            "a wrong secret never authenticates"
        );
    }
    assert!(
        manager
            .verify(&record.key, &secret)
            .await
            .unwrap()
            .is_none(),
        "the budget is exhausted: the right secret is refused too"
    );
}
