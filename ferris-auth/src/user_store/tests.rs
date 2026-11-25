use super::*;
use ferris_core::secret::is_secret_envelope;
use ferris_db::{Ident, Stmt};

async fn db() -> Db {
    ferris_db::memory_db().await.unwrap()
}

/// `value` quoted for the store's dialect.
fn quoted(store: &UserStore, value: &str) -> String {
    Ident::new(value)
        .expect("a literal identifier")
        .quote(store.db().dialect())
}

/// Write `mfa_secret` straight into the column, bypassing the engine.
///
/// This is how a row written before the field was declared `Secret` looks: the
/// seed sits in the clear, with no envelope.
async fn write_raw_mfa_secret(store: &UserStore, id: &str, value: &str) {
    store
        .db()
        .execute(&Stmt::new(
            format!(
                "UPDATE {} SET {} = ? WHERE {} = ?",
                quoted(store, &ferris_document::table_name("User").unwrap()),
                quoted(store, "mfa_secret"),
                quoted(store, "name"),
            ),
            vec![Value::from(value), Value::from(id)],
        ))
        .await
        .unwrap();
}

/// The `mfa_secret` column exactly as stored.
async fn stored_mfa_secret(store: &UserStore, id: &str) -> Option<String> {
    let row = store
        .db()
        .fetch_optional(&Stmt::new(
            format!(
                "SELECT {} FROM {} WHERE {} = ?",
                quoted(store, "mfa_secret"),
                quoted(store, &ferris_document::table_name("User").unwrap()),
                quoted(store, "name"),
            ),
            vec![Value::from(id)],
        ))
        .await
        .unwrap()?;
    row.opt_str("mfa_secret")
}

#[tokio::test]
async fn create_and_login_roundtrip() {
    let store = UserStore::new(db().await);
    store.init().await.unwrap();
    let user = store
        .create(
            "admin@local",
            "Administrator",
            Some("admin"),
            UserType::SystemUser,
            "ferrisAdmin123",
            &["System Manager".to_string()],
        )
        .await
        .unwrap();
    assert_eq!(user.roles, vec!["System Manager".to_string()]);

    let by_email = store.find_by_login("admin@local").await.unwrap().unwrap();
    assert_eq!(by_email.id, user.id);
    let by_user = store.find_by_login("admin").await.unwrap().unwrap();
    assert_eq!(by_user.id, user.id);

    assert!(
        store
            .verify_login("admin", "ferrisAdmin123")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .verify_login("admin", "wrong")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .verify_login("nobody", "ferrisAdmin123")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn ensure_admin_is_idempotent() {
    let store = UserStore::new(db().await);
    let a = store
        .ensure_admin(
            "admin@local",
            "ferrisAdmin123",
            &["System Manager".to_string()],
        )
        .await
        .unwrap();
    let b = store
        .ensure_admin(
            "admin@local",
            "ferrisAdmin123",
            &["System Manager".to_string()],
        )
        .await
        .unwrap();
    assert_eq!(a.id, b.id);
}

/// The seed lives sealed at rest now, but a row written before the field was
/// declared `Secret` still holds it in the clear. That value must stay readable
/// (and usable for a login) until `set_mfa` replaces it with an envelope.
#[tokio::test]
async fn a_legacy_plaintext_mfa_secret_is_readable_and_then_sealed() {
    const LEGACY_SEED: &str = "JBSWY3DPEHPK3PXP";

    install_test_keyring();
    let store = UserStore::new(db().await);
    store.init().await.unwrap();
    let user = store
        .create(
            "legacy-mfa@local",
            "Legacy Mfa",
            None,
            UserType::SystemUser,
            "ferrisAdmin123",
            &["User".to_string()],
        )
        .await
        .unwrap();

    write_raw_mfa_secret(&store, &user.id, LEGACY_SEED).await;
    assert_eq!(
        stored_mfa_secret(&store, &user.id).await.as_deref(),
        Some(LEGACY_SEED)
    );

    // Read back through the decrypting path: the legacy value comes back as-is.
    assert_eq!(
        store.mfa_secret(&user.id).await.unwrap().as_deref(),
        Some(LEGACY_SEED)
    );
    // The login record resolves the same seed, so an existing MFA account keeps
    // signing in.
    let record = store
        .login_record("legacy-mfa@local")
        .await
        .unwrap()
        .expect("the account is enabled");
    assert_eq!(record.mfa_secret.as_deref(), Some(LEGACY_SEED));

    // The next write seals it; the plaintext is still what the reader returns.
    store
        .set_mfa(&user.id, false, Some(LEGACY_SEED), &[])
        .await
        .unwrap();
    let stored = stored_mfa_secret(&store, &user.id)
        .await
        .expect("the seed is still stored");
    assert!(
        is_secret_envelope(&stored),
        "set_mfa is expected to store an envelope, got {stored}"
    );
    assert!(!stored.contains(LEGACY_SEED));
    assert_eq!(
        store.mfa_secret(&user.id).await.unwrap().as_deref(),
        Some(LEGACY_SEED)
    );
}

/// The TOTP step is spent in the database, so a replay is refused by every
/// replica, not just the process that saw the first attempt.
#[tokio::test]
async fn a_totp_step_is_spent_once_and_persisted() {
    let store = UserStore::new(db().await);
    store.init().await.unwrap();
    let user = store
        .create(
            "totp-step@local",
            "Totp Step",
            None,
            UserType::SystemUser,
            "ferrisAdmin123",
            &["User".to_string()],
        )
        .await
        .unwrap();

    assert!(store.claim_totp_step(&user.id, 100).await.unwrap());
    assert!(
        !store.claim_totp_step(&user.id, 100).await.unwrap(),
        "the same step is refused the second time"
    );
    assert!(
        !store.claim_totp_step(&user.id, 99).await.unwrap(),
        "an earlier step cannot come back after a later one"
    );
    assert!(
        store.claim_totp_step(&user.id, 101).await.unwrap(),
        "the next step is still fresh"
    );

    // A second store over the same database reads the same column: the claim is
    // shared state, not a process-local map.
    let other = UserStore::new(store.db().clone());
    assert!(!other.claim_totp_step(&user.id, 101).await.unwrap());
    assert!(other.claim_totp_step(&user.id, 102).await.unwrap());

    // An unknown account is refused, never created by the claim.
    assert!(!other.claim_totp_step("no-such-user", 200).await.unwrap());
}

#[tokio::test]
async fn mfa_secret_and_backup_codes_round_trip() {
    install_test_keyring();
    let store = UserStore::new(db().await);
    store.init().await.unwrap();
    let user = store
        .create(
            "mfa@local",
            "Mfa User",
            None,
            UserType::SystemUser,
            "ferrisAdmin123",
            &["User".to_string()],
        )
        .await
        .unwrap();

    // Enable flow: the secret persists while the flag is still off.
    assert!(!store.mfa_enabled(&user.id).await.unwrap());
    let codes = ["CODE-1".to_string(), "CODE-2".to_string()];
    let hashes: Vec<String> = codes
        .iter()
        .map(|c| UserStore::hash_backup_code(c).unwrap())
        .collect();
    store
        .set_mfa(&user.id, false, Some("base32secret"), &hashes)
        .await
        .unwrap();
    assert_eq!(
        store.mfa_secret(&user.id).await.unwrap().as_deref(),
        Some("base32secret")
    );
    assert!(!store.mfa_enabled(&user.id).await.unwrap());

    // Activation flips the flag.
    store.set_mfa_enabled(&user.id, true).await.unwrap();
    assert!(store.mfa_enabled(&user.id).await.unwrap());

    // Backup codes are single-use: the first pass consumes, the second fails.
    assert!(
        store
            .consume_mfa_backup_code(&user.id, "CODE-1")
            .await
            .unwrap()
    );
    assert!(
        !store
            .consume_mfa_backup_code(&user.id, "CODE-1")
            .await
            .unwrap()
    );
    assert!(
        store
            .consume_mfa_backup_code(&user.id, "CODE-2")
            .await
            .unwrap()
    );
    assert!(
        !store
            .consume_mfa_backup_code(&user.id, "nope")
            .await
            .unwrap()
    );

    // Disable clears the secret.
    store.set_mfa(&user.id, false, None, &[]).await.unwrap();
    assert!(store.mfa_secret(&user.id).await.unwrap().is_none());
}

/// AUTH8: a backup code is stored as an Argon2 hash, not an unsalted
/// SHA-256 hex digest. A 41-bit code behind a fast, unstretched digest is
/// brute-forceable offline, so a leaked store used to leak usable codes.
#[test]
fn backup_codes_are_argon2_hashed_not_sha256() {
    let hash = UserStore::hash_backup_code("ABCD1234").unwrap();

    // Argon2id PHC string, not a 64-char hex digest.
    assert!(hash.starts_with("$argon2id$"), "{hash}");
    assert_ne!(hash.len(), 64, "must not be a plain SHA-256 hex digest");
    assert!(
        crate::verify_password("ABCD1234", &hash).unwrap(),
        "the stored hash must verify the code"
    );
    assert!(!crate::verify_password("WRONG123", &hash).unwrap());

    // Two hashes of the same code differ: the salt is random.
    assert_ne!(hash, UserStore::hash_backup_code("ABCD1234").unwrap());
}

#[tokio::test]
async fn record_login_counts_and_stamps() {
    let store = UserStore::new(db().await);
    store.init().await.unwrap();
    let user = store
        .create(
            "login@local",
            "Login User",
            None,
            UserType::SystemUser,
            "ferrisAdmin123",
            &["User".to_string()],
        )
        .await
        .unwrap();
    assert!(user.last_login.is_none());
    assert_eq!(user.login_count, 0);

    store.record_login(&user.id).await.unwrap();
    store.record_login(&user.id).await.unwrap();
    let reloaded = store.find_by_id(&user.id).await.unwrap().unwrap();
    assert_eq!(reloaded.login_count, 2);
    assert!(reloaded.last_login.is_some());

    // Unknown users are a no-op, not an error.
    store.record_login("no-such-user").await.unwrap();
}

#[tokio::test]
async fn to_user_maps_identity() {
    let store = UserStore::new(db().await);
    store.init().await.unwrap();
    let u = store
        .create(
            "x@local",
            "Somebody",
            Some("somebody"),
            UserType::SystemUser,
            "ferrisAdmin123",
            &["User".to_string()],
        )
        .await
        .unwrap();
    let user: User = u.to_user();
    assert_eq!(user.email, "x@local");
    assert!(user.has_role("User"));
    assert!(user.enabled);
}

#[tokio::test]
async fn the_account_language_round_trips_and_clears() {
    let store = UserStore::new(db().await);
    let user = store
        .create(
            "ada@example.com",
            "Ada Lovelace",
            Some("ada"),
            crate::UserType::SystemUser,
            "Secret123!",
            &["System Manager".to_string()],
        )
        .await
        .expect("the user is created");
    assert_eq!(user.language, None, "a new account follows the browser");

    store
        .set_language(&user.id, Some("es"))
        .await
        .expect("the language is stored");
    let stored = store
        .find_by_id(&user.id)
        .await
        .expect("the user is readable")
        .expect("the user exists");
    assert_eq!(stored.language.as_deref(), Some("es"));
    assert_eq!(
        stored
            .public_json()
            .get("language")
            .and_then(|v| v.as_str()),
        Some("es")
    );

    store
        .set_language(&user.id, None)
        .await
        .expect("the preference is cleared");
    let cleared = store
        .find_by_id(&user.id)
        .await
        .expect("the user is readable")
        .expect("the user exists");
    assert_eq!(cleared.language, None);
}

/// Two accounts may not share an email or a username.
///
/// The login path resolves by `email` (then `username`) and takes the first
/// match, and password reset mints a token for that same first match: a
/// duplicate address would make sign-in and reset resolve an account the caller
/// does not own. The unique indexes are the enforcement; the engine reports the
/// constraint as `AlreadyExists`.
#[tokio::test]
async fn a_duplicate_email_or_username_is_refused() {
    let store = UserStore::new(db().await);
    store.init().await.unwrap();
    store
        .create(
            "taken@example.com",
            "First",
            Some("first"),
            super::UserType::SystemUser,
            "Secret-Password1",
            &[],
        )
        .await
        .unwrap();

    let duplicate_email = store
        .create(
            "taken@example.com",
            "Second",
            Some("second"),
            super::UserType::SystemUser,
            "Secret-Password2",
            &[],
        )
        .await;
    assert!(
        matches!(
            duplicate_email,
            Err(Error::Document(DocumentError::AlreadyExists { .. }))
        ),
        "a duplicate email must be refused, got {duplicate_email:?}"
    );

    let duplicate_username = store
        .create(
            "other@example.com",
            "Third",
            Some("first"),
            super::UserType::SystemUser,
            "Secret-Password3",
            &[],
        )
        .await;
    assert!(
        matches!(
            duplicate_username,
            Err(Error::Document(DocumentError::AlreadyExists { .. }))
        ),
        "a duplicate username must be refused, got {duplicate_username:?}"
    );

    // Two accounts without a username are still fine: `NULL` is not a value.
    for (email, name) in [("a@example.com", "A"), ("b@example.com", "B")] {
        store
            .create(
                email,
                name,
                None,
                super::UserType::SystemUser,
                "Secret-Password4",
                &[],
            )
            .await
            .unwrap();
    }
}
