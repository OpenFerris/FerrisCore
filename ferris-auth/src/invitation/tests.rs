//! Tests for the invitation flow.

use super::*;
use crate::UserType;
use ferris_core::{AuthError, Error};

async fn store() -> InvitationStore {
    let store = InvitationStore::new(ferris_db::memory_db().await.unwrap());
    store.init().await.unwrap();
    store.users().init().await.unwrap();
    store
}

/// The `token_hash` column exactly as stored.
async fn token_hash(store: &InvitationStore, id: &str) -> Option<String> {
    let table = store.table().unwrap();
    let name = store.column("name").unwrap();
    let token = store.column("token_hash").unwrap();
    let row = store
        .db
        .fetch_optional(&Stmt::new(
            format!("SELECT {token} FROM {table} WHERE {name} = ?"),
            vec![Value::from(id)],
        ))
        .await
        .unwrap()?;
    row.opt_str("token_hash")
}

#[tokio::test]
async fn an_invitation_creates_a_login_capable_user_exactly_once() {
    let store = store().await;
    let roles = vec!["User".to_string()];
    let (invitation, token) = store
        .create(
            "invitee@local",
            Some("Invited Name"),
            &roles,
            Duration::hours(48),
            Some("admin-id"),
        )
        .await
        .unwrap();
    assert_eq!(invitation.email, "invitee@local");
    assert_eq!(invitation.full_name.as_deref(), Some("Invited Name"));
    assert_eq!(invitation.roles, roles);
    assert_eq!(invitation.invited_by.as_deref(), Some("admin-id"));
    assert!(!invitation.accepted);
    assert!(invitation.expires_at.is_some());
    assert!(!token.is_empty());
    // The token is stored only as a digest.
    assert_eq!(
        token_hash(&store, &invitation.id).await.as_deref(),
        Some(crate::crypto::sha256_hex(token.as_bytes()).as_str())
    );
    assert_ne!(token_hash(&store, &invitation.id).await.unwrap(), token);

    let listed = store.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, invitation.id);
    assert!(!listed[0].accepted);

    let user = store
        .accept(&token, "Ada Lovelace", "Secret123!")
        .await
        .unwrap()
        .expect("the token is live");
    assert_eq!(user.email, "invitee@local");
    assert_eq!(user.full_name, "Ada Lovelace");
    assert_eq!(user.roles, roles);
    assert!(
        store
            .users()
            .verify_login("invitee@local", "Secret123!")
            .await
            .unwrap()
            .is_some(),
        "the invited account can sign in"
    );

    let accepted = store.list().await.unwrap();
    assert_eq!(accepted.len(), 1);
    assert!(accepted[0].accepted);
    assert!(accepted[0].accepted_at.is_some());
    assert!(
        token_hash(&store, &invitation.id).await.is_none(),
        "acceptance clears the token digest"
    );
    assert!(
        store
            .accept(&token, "Impostor", "OtherPassword123!")
            .await
            .unwrap()
            .is_none(),
        "the token cannot be used twice"
    );
}

#[tokio::test]
async fn an_invitation_name_is_the_fallback() {
    let store = store().await;
    let (invitation, token) = store
        .create(
            "fallback@local",
            Some("Invited Name"),
            &[],
            Duration::hours(1),
            None,
        )
        .await
        .unwrap();
    let user = store
        .accept(&token, "   ", "Secret123!")
        .await
        .unwrap()
        .expect("the token is live");
    assert_eq!(user.full_name, "Invited Name");
    assert_eq!(invitation.email, user.email);
}

#[tokio::test]
async fn an_expired_invitation_is_refused() {
    let store = store().await;
    let (invitation, token) = store
        .create("late@local", None, &[], Duration::seconds(-1), None)
        .await
        .unwrap();
    assert!(
        store
            .accept(&token, "Late Person", "Secret123!")
            .await
            .unwrap()
            .is_none(),
        "an expired invitation creates nothing"
    );
    assert!(!store.list().await.unwrap()[0].accepted);
    assert!(token_hash(&store, &invitation.id).await.is_some());
    assert!(
        store
            .users()
            .find_by_login("late@local")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_duplicate_email_is_refused_and_the_invitation_survives() {
    let store = store().await;
    store
        .users()
        .create(
            "dup@local",
            "Existing",
            None,
            UserType::SystemUser,
            "Secret123!",
            &[],
        )
        .await
        .unwrap();
    let (invitation, token) = store
        .create("dup@local", None, &[], Duration::hours(1), None)
        .await
        .unwrap();

    assert!(
        store
            .accept(&token, "Second Account", "OtherPassword123!")
            .await
            .unwrap()
            .is_none(),
        "an existing address is not duplicated"
    );
    assert!(
        !store.list().await.unwrap()[0].accepted,
        "the pending invitation is not consumed"
    );
    assert!(token_hash(&store, &invitation.id).await.is_some());
}

#[tokio::test]
async fn an_unknown_token_is_refused() {
    let store = store().await;
    store
        .create("known@local", None, &[], Duration::hours(1), None)
        .await
        .unwrap();
    for token in ["", "   ", "not-a-real-token"] {
        assert!(
            store
                .accept(token, "Nobody", "Secret123!")
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(
        store
            .users()
            .find_by_login("nobody@local")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn revoke_kills_the_token_and_the_row() {
    let store = store().await;
    let (invitation, token) = store
        .create("revoked@local", None, &[], Duration::hours(1), None)
        .await
        .unwrap();
    store.revoke(&invitation.id).await.unwrap();
    assert!(store.list().await.unwrap().is_empty());
    assert!(
        store
            .accept(&token, "Revoked Person", "Secret123!")
            .await
            .unwrap()
            .is_none()
    );

    match store.revoke(&invitation.id).await {
        Err(Error::Auth(AuthError::NotFound(_))) => {}
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn create_requires_an_email() {
    let store = store().await;
    for email in ["", "   "] {
        assert!(
            store
                .create(email, None, &[], Duration::hours(1), None)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn a_rejected_password_releases_the_invitation() {
    let store = store().await;
    let (invitation, token) = store
        .create("retry@local", None, &[], Duration::hours(1), None)
        .await
        .unwrap();

    assert!(
        store.accept(&token, "Retry Person", "short").await.is_err(),
        "the password policy rejects the password"
    );
    assert!(
        !store.list().await.unwrap()[0].accepted,
        "the claim is released"
    );
    assert_eq!(
        token_hash(&store, &invitation.id).await.as_deref(),
        Some(crate::crypto::sha256_hex(token.as_bytes()).as_str()),
        "the same token can be retried"
    );
    assert!(
        store
            .accept(&token, "Retry Person", "Secret123!")
            .await
            .unwrap()
            .is_some()
    );
}
