//! Tests for the password-reset and email-verification flows.

use super::*;
use crate::UserType;
use ferris_db::{Db, Stmt};

async fn db() -> Db {
    ferris_db::memory_db().await.unwrap()
}

/// An initialised store with one enabled account.
async fn store_with_user(email: &str, password: &str) -> (UserStore, StoredUser) {
    let store = UserStore::new(db().await);
    store.init().await.unwrap();
    let user = store
        .create(
            email,
            "Test User",
            None,
            UserType::SystemUser,
            password,
            &["User".to_string()],
        )
        .await
        .unwrap();
    (store, user)
}

/// A `User` text column exactly as stored, bypassing the read mask.
async fn stored_column(store: &UserStore, id: &str, column: &str) -> Option<String> {
    let table = store.user_table().unwrap();
    let name = store.user_column("name").unwrap();
    let quoted = store.user_column(column).unwrap();
    let row = store
        .db()
        .fetch_optional(&Stmt::new(
            format!("SELECT {quoted} FROM {table} WHERE {name} = ?"),
            vec![Value::from(id)],
        ))
        .await
        .unwrap()?;
    row.opt_str(column)
}

/// Disable an account through the engine (the desk's own path).
async fn disable(store: &UserStore, id: &str) {
    let mut fields = Map::new();
    fields.insert("enabled".to_string(), Value::from(false));
    store.patch(id, fields).await.unwrap();
}

/// The `email_verified` flag as stored.
async fn email_verified(store: &UserStore, id: &str) -> bool {
    store
        .load(id)
        .await
        .unwrap()
        .and_then(|document| document.get("email_verified").and_then(Value::as_bool))
        .unwrap_or(false)
}

#[tokio::test]
async fn reset_happy_path_sets_a_new_password_and_consumes_the_token() {
    let (store, user) = store_with_user("reset@local", "OldPassword123!").await;
    let service = PasswordResetService::new(store.clone());

    let token = service
        .request_reset("reset@local", Duration::hours(1))
        .await
        .unwrap()
        .expect("a known account gets a token");
    assert_eq!(token.user_id, user.id);
    assert!(!token.token.is_empty());
    // Only the digest is persisted, never the token.
    let stored = stored_column(&store, &user.id, RESET_KEY_FIELD)
        .await
        .expect("the digest is stored");
    assert_eq!(stored, crate::crypto::sha256_hex(token.token.as_bytes()));
    assert_ne!(stored, token.token);

    let confirmed = service
        .confirm_reset(&token.token, "NewPassword123!")
        .await
        .unwrap()
        .expect("the token is live");
    assert_eq!(confirmed.id, user.id);
    assert!(
        store
            .verify_login("reset@local", "NewPassword123!")
            .await
            .unwrap()
            .is_some(),
        "the new password works"
    );
    assert!(
        store
            .verify_login("reset@local", "OldPassword123!")
            .await
            .unwrap()
            .is_none(),
        "the old password no longer works"
    );
    assert!(
        stored_column(&store, &user.id, RESET_KEY_FIELD)
            .await
            .is_none(),
        "a successful confirmation clears the digest"
    );
    assert!(
        stored_column(&store, &user.id, RESET_EXPIRES_FIELD)
            .await
            .is_none(),
        "a successful confirmation clears the deadline"
    );
}

#[tokio::test]
async fn a_reset_token_is_single_use() {
    let (store, _user) = store_with_user("reuse@local", "OldPassword123!").await;
    let service = PasswordResetService::new(store.clone());
    let token = service
        .request_reset("reuse@local", Duration::hours(1))
        .await
        .unwrap()
        .expect("a known account gets a token");

    assert!(
        service
            .confirm_reset(&token.token, "NewPassword123!")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        service
            .confirm_reset(&token.token, "AnotherPassword123!")
            .await
            .unwrap()
            .is_none(),
        "the second confirmation finds nothing"
    );
    assert!(
        store
            .verify_login("reuse@local", "NewPassword123!")
            .await
            .unwrap()
            .is_some(),
        "the replay did not overwrite the password"
    );
}

#[tokio::test]
async fn an_expired_reset_token_is_refused() {
    let (store, user) = store_with_user("expired@local", "OldPassword123!").await;
    let service = PasswordResetService::new(store.clone());
    let token = service
        .request_reset("expired@local", Duration::seconds(-1))
        .await
        .unwrap()
        .expect("issuing does not depend on the deadline");

    assert!(
        service
            .confirm_reset(&token.token, "NewPassword123!")
            .await
            .unwrap()
            .is_none(),
        "an expired token is refused"
    );
    // The token was not consumed by the failed attempt; it is dead by deadline.
    assert!(
        stored_column(&store, &user.id, RESET_KEY_FIELD)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn a_new_reset_token_replaces_the_previous_one() {
    let (store, _user) = store_with_user("replace@local", "OldPassword123!").await;
    let service = PasswordResetService::new(store.clone());
    let first = service
        .request_reset("replace@local", Duration::hours(1))
        .await
        .unwrap()
        .expect("first token");
    let second = service
        .request_reset("replace@local", Duration::hours(1))
        .await
        .unwrap()
        .expect("second token");

    assert!(
        service
            .confirm_reset(&first.token, "NewPassword123!")
            .await
            .unwrap()
            .is_none(),
        "the replaced token is dead"
    );
    assert!(
        service
            .confirm_reset(&second.token, "NewPassword123!")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn unknown_and_disabled_accounts_are_indistinguishable() {
    let (store, user) = store_with_user("off@local", "OldPassword123!").await;
    let service = PasswordResetService::new(store.clone());

    assert!(
        service
            .request_reset("nobody@local", Duration::hours(1))
            .await
            .unwrap()
            .is_none(),
        "an unknown address is Ok(None), never an error"
    );
    assert!(
        service
            .confirm_reset("not-a-real-token", "NewPassword123!")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .confirm_reset("   ", "NewPassword123!")
            .await
            .unwrap()
            .is_none()
    );

    // A token issued while the account was enabled dies with the account.
    let token = service
        .request_reset("off@local", Duration::hours(1))
        .await
        .unwrap()
        .expect("the account is enabled");
    disable(&store, &user.id).await;
    assert!(
        service
            .request_reset("off@local", Duration::hours(1))
            .await
            .unwrap()
            .is_none(),
        "a disabled account answers exactly like an unknown one"
    );
    assert!(
        service
            .confirm_reset(&token.token, "NewPassword123!")
            .await
            .unwrap()
            .is_none(),
        "a disabled account cannot be reset"
    );
}

#[tokio::test]
async fn a_reset_is_requested_by_address_not_by_username() {
    let store = UserStore::new(db().await);
    store.init().await.unwrap();
    store
        .create(
            "owner@local",
            "Owner",
            Some("owner"),
            UserType::SystemUser,
            "OldPassword123!",
            &[],
        )
        .await
        .unwrap();
    let service = PasswordResetService::new(store.clone());

    assert!(
        service
            .request_reset("owner", Duration::hours(1))
            .await
            .unwrap()
            .is_none(),
        "the username resolves no address to mail"
    );
    assert!(
        service
            .request_reset("owner@local", Duration::hours(1))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn an_empty_address_is_refused_without_a_lookup() {
    let (store, _user) = store_with_user("empty@local", "OldPassword123!").await;
    let service = PasswordResetService::new(store);
    for address in ["", "   "] {
        assert!(
            service
                .request_reset(address, Duration::hours(1))
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn verification_happy_path_marks_the_address_and_consumes_the_token() {
    let (store, user) = store_with_user("verify@local", "OldPassword123!").await;
    let service = EmailVerificationService::new(store.clone());
    assert!(!email_verified(&store, &user.id).await);

    let token = service
        .request_verification(&user.id, Duration::hours(1))
        .await
        .unwrap()
        .expect("an unverified account gets a token");
    assert_eq!(token.user_id, user.id);
    assert_eq!(
        stored_column(&store, &user.id, VERIFICATION_KEY_FIELD)
            .await
            .expect("the digest is stored"),
        crate::crypto::sha256_hex(token.token.as_bytes())
    );

    let confirmed = service
        .confirm_verification(&token.token)
        .await
        .unwrap()
        .expect("the token is live");
    assert_eq!(confirmed.id, user.id);
    assert!(email_verified(&store, &user.id).await);
    assert!(
        stored_column(&store, &user.id, VERIFICATION_KEY_FIELD)
            .await
            .is_none(),
        "a successful confirmation clears the digest"
    );

    assert!(
        service
            .confirm_verification(&token.token)
            .await
            .unwrap()
            .is_none(),
        "the token cannot be reused"
    );
    assert!(
        service
            .request_verification(&user.id, Duration::hours(1))
            .await
            .unwrap()
            .is_none(),
        "a verified account has nothing left to prove"
    );
}

#[tokio::test]
async fn an_expired_or_unknown_verification_token_is_refused() {
    let (store, user) = store_with_user("expired-verify@local", "OldPassword123!").await;
    let service = EmailVerificationService::new(store.clone());

    let token = service
        .request_verification(&user.id, Duration::seconds(-1))
        .await
        .unwrap()
        .expect("issuing does not depend on the deadline");
    assert!(
        service
            .confirm_verification(&token.token)
            .await
            .unwrap()
            .is_none()
    );
    assert!(!email_verified(&store, &user.id).await);

    assert!(
        service
            .confirm_verification("not-a-real-token")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .request_verification("no-such-user", Duration::hours(1))
            .await
            .unwrap()
            .is_none(),
        "an unknown account is Ok(None)"
    );
}

#[tokio::test]
async fn a_disabled_account_cannot_start_verification() {
    let (store, user) = store_with_user("disabled-verify@local", "OldPassword123!").await;
    disable(&store, &user.id).await;
    let service = EmailVerificationService::new(store);
    assert!(
        service
            .request_verification(&user.id, Duration::hours(1))
            .await
            .unwrap()
            .is_none()
    );
}
