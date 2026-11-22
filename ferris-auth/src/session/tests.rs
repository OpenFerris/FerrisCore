use super::*;

#[tokio::test]
async fn test_session_creation() {
    let manager = SessionManager::default();
    let session = manager
        .create_session("user1".to_string(), None, None)
        .await
        .unwrap();

    assert_eq!(session.user_id, "user1");
    assert!(!session.is_expired());
    assert!(!session.csrf_token.is_empty());
}

#[tokio::test]
async fn test_session_touch() {
    let manager = SessionManager::default();
    let session = manager
        .create_session("user1".to_string(), None, None)
        .await
        .unwrap();
    let session_id = session.id.clone();

    let touched = manager.touch_session(&session_id).await.unwrap();
    assert!(touched.is_some());
    assert!(touched.unwrap().last_activity >= session.last_activity);
}

#[tokio::test]
async fn test_session_deletion() {
    let manager = SessionManager::default();
    let session = manager
        .create_session("user1".to_string(), None, None)
        .await
        .unwrap();
    let session_id = session.id.clone();

    manager.delete_session(&session_id).await.unwrap();
    let result = manager.get_session(&session_id).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_max_sessions() {
    let config = SessionConfig {
        max_sessions_per_user: 2,
        ..Default::default()
    };
    let manager = SessionManager::new(config);

    let _s1 = manager
        .create_session("user1".to_string(), None, None)
        .await
        .unwrap();
    let _s2 = manager
        .create_session("user1".to_string(), None, None)
        .await
        .unwrap();
    // A third login is refused, and the two live sessions are untouched: the
    // old behaviour evicted the oldest and stole its tab.
    let err = manager
        .create_session("user1".to_string(), None, None)
        .await
        .expect_err("the third session exceeds max_sessions_per_user");
    assert!(
        matches!(
            err,
            ferris_core::Error::Auth(ferris_core::AuthError::TooManySessions(2))
        ),
        "{err:?}"
    );

    let user_sessions = manager.get_user_sessions("user1").await.unwrap();
    assert_eq!(user_sessions.len(), 2); // Nothing was evicted
}

/// A20: a session's CSRF token validates only against that very session.
/// The same-length wrong token (every byte changed) and a different-length
/// one are both refused, and an unknown session id never validates.
#[tokio::test]
async fn csrf_tokens_validate_only_against_their_own_session() {
    let manager = SessionManager::default();
    let session = manager
        .create_session("user1".to_string(), None, None)
        .await
        .unwrap();

    assert!(
        manager
            .validate_csrf_token(&session.id, &session.csrf_token)
            .await
            .unwrap(),
        "the session's own token is valid"
    );

    // Same length, every character swapped: rejected (the comparison is
    // constant-time over equal-length buffers, not string equality).
    let wrong: String = session
        .csrf_token
        .chars()
        .map(|c| if c == 'f' { '0' } else { 'f' })
        .collect();
    assert_eq!(wrong.len(), session.csrf_token.len());
    assert!(
        !manager
            .validate_csrf_token(&session.id, &wrong)
            .await
            .unwrap()
    );

    // Different length: rejected before any byte is compared.
    assert!(
        !manager
            .validate_csrf_token(&session.id, "short")
            .await
            .unwrap()
    );

    // No such session: nothing to validate against.
    assert!(
        !manager
            .validate_csrf_token("no-such-session", &session.csrf_token)
            .await
            .unwrap()
    );
}

/// A20: freshly created sessions each carry their own token — one
/// session's token must never open another session.
#[tokio::test]
async fn csrf_tokens_are_per_session() {
    let manager = SessionManager::default();
    let a = manager
        .create_session("user1".to_string(), None, None)
        .await
        .unwrap();
    let b = manager
        .create_session("user1".to_string(), None, None)
        .await
        .unwrap();

    assert_ne!(a.csrf_token, b.csrf_token);
    assert!(
        !manager
            .validate_csrf_token(&b.id, &a.csrf_token)
            .await
            .unwrap()
    );
}

/// The engine is the source of truth: a session persisted by one manager is
/// visible to a second manager that shares the database but has an empty
/// cache, including its `permlevel` 1 CSRF token.
#[tokio::test]
async fn engine_backed_sessions_persist_across_managers() {
    let db = ferris_db::memory_db().await.unwrap();

    let first = SessionManager::with_db(SessionConfig::default(), db.clone());
    first.init().await.unwrap();
    let session = first
        .create_session(
            "user@example.com".to_string(),
            Some("10.0.0.1".to_string()),
            Some("agent/1".to_string()),
        )
        .await
        .unwrap();

    // A fresh manager with no cache still reads the document back.
    let second = SessionManager::with_db(SessionConfig::default(), db);
    let loaded = second.get_session(&session.id).await.unwrap().unwrap();
    assert_eq!(loaded.user_id, "user@example.com");
    assert_eq!(loaded.ip_address.as_deref(), Some("10.0.0.1"));
    assert_eq!(loaded.csrf_token, session.csrf_token);
    assert!(
        second
            .validate_csrf_token(&session.id, &session.csrf_token)
            .await
            .unwrap()
    );

    // A user lookup goes through the engine too, and deletion removes the
    // document for good.
    assert_eq!(
        second
            .get_user_sessions("user@example.com")
            .await
            .unwrap()
            .len(),
        1
    );
    second.delete_session(&session.id).await.unwrap();
    assert!(second.get_session(&session.id).await.unwrap().is_none());
}
