use super::*;

async fn db() -> Db {
    ferris_db::memory_db().await.unwrap()
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

#[tokio::test]
async fn mfa_secret_and_backup_codes_round_trip() {
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
