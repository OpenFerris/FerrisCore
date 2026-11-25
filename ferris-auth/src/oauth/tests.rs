use super::state_store::PendingState;
use super::*;
use ferris_core::Error;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[test]
fn test_oauth_config() {
    let config = OAuthConfig::default();
    assert!(config.providers.is_empty());
    assert!(config.state_store.enabled);
}

async fn memory_db() -> ferris_db::Db {
    ferris_db::memory_db().await.expect("in-memory db")
}

fn provider_with_token_url(token_url: String) -> OAuthProviderConfig {
    OAuthProviderConfig {
        name: "test".to_string(),
        client_id: "client".to_string(),
        client_secret: "secret".to_string(),
        auth_url: "https://provider.example/auth".to_string(),
        token_url,
        user_info_url: None,
        device_auth_url: None,
        scopes: vec!["openid".to_string()],
        pkce_enabled: false,
        pkce_method: PkceMethod::S256,
        redirect_uris: vec!["https://app.example/callback".to_string()],
        require_verified_email: true,
        oidc: false,
    }
}

fn config_with(provider: OAuthProviderConfig) -> OAuthConfig {
    OAuthConfig {
        providers: vec![provider],
        state_store: StateStoreConfig {
            enabled: true,
            ttl_seconds: 600,
        },
    }
}

/// AUTH4: a disabled state store must not brick the callback — the provider
/// named in the path is used, and no state entry is required. (The old
/// `callback` always required the entry, so `enabled = false` made every
/// login fail with "Invalid or expired state".)
#[tokio::test]
async fn a_disabled_state_store_still_completes_the_callback() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "stateless-access",
            "token_type": "Bearer",
            "expires_in": 600
        })))
        .mount(&server)
        .await;

    let provider = provider_with_token_url(format!("{}/token", server.uri()));
    let config = OAuthConfig {
        providers: vec![provider],
        state_store: StateStoreConfig {
            enabled: false,
            ttl_seconds: 600,
        },
    };
    let mut manager = OAuthManager::new(config).unwrap();

    // `authorize` succeeds but stores nothing.
    let (_, oauth_state) = manager.authorize("test").await.unwrap();
    assert_eq!(
        manager.state_store.memory_len(),
        Some(0),
        "disabled store keeps nothing"
    );

    // The callback still resolves the provider from the request path.
    let tokens = manager
        .callback("test", oauth_state, "code".to_string())
        .await
        .expect("stateless mode completes the exchange");
    assert_eq!(tokens.access_token, "stateless-access");
}

/// AUTH4: `authorize` prunes expired states, so an abandoned login cannot
/// grow the in-memory store without bound.
#[tokio::test]
async fn authorize_prunes_expired_states() {
    let mut manager = OAuthManager::new(config_with(provider_with_token_url(
        "https://provider.example/token".to_string(),
    )))
    .unwrap();

    // Seed an already-expired entry, then run a fresh authorize.
    manager
        .state_store
        .put(
            "stale",
            PendingState {
                provider: "test".to_string(),
                pkce_verifier: None,
                expires_at: chrono::Utc::now() - chrono::Duration::seconds(1),
            },
        )
        .await
        .unwrap();
    let (_, state) = manager.authorize("test").await.unwrap();

    assert!(!manager.state_store.memory_contains("stale"));
    assert!(manager.state_store.memory_contains(&state));
}

/// The DB store is single use even across two managers sharing the database:
/// the state is removed before validation, so the second callback cannot win.
#[tokio::test]
async fn db_states_are_single_use() {
    let manager_config = config_with(provider_with_token_url(
        "https://provider.example/token".to_string(),
    ));
    let mut manager = OAuthManager::with_db(manager_config, memory_db().await).unwrap();

    let (_, state) = manager.authorize("test").await.unwrap();
    manager
        .take_pending("test", &state)
        .await
        .expect("the first callback wins");
    let error = manager
        .take_pending("test", &state)
        .await
        .expect_err("the second callback must lose");
    assert!(matches!(
        error,
        Error::Auth(ferris_core::AuthError::TokenInvalid(_))
    ));
}

/// An expired DB row is refused as expired and removed, so a retry is invalid.
#[tokio::test]
async fn expired_db_states_are_refused_and_removed() {
    let manager_config = config_with(provider_with_token_url(
        "https://provider.example/token".to_string(),
    ));
    let mut manager = OAuthManager::with_db(manager_config, memory_db().await).unwrap();
    manager
        .state_store
        .put(
            "expired",
            PendingState {
                provider: "test".to_string(),
                pkce_verifier: None,
                expires_at: chrono::Utc::now() - chrono::Duration::seconds(1),
            },
        )
        .await
        .unwrap();

    let error = manager
        .take_pending("test", "expired")
        .await
        .expect_err("an expired state is refused");
    assert!(matches!(
        error,
        Error::Auth(ferris_core::AuthError::TokenExpired)
    ));
    let error = manager
        .take_pending("test", "expired")
        .await
        .expect_err("the row is gone after the refusal");
    assert!(matches!(
        error,
        Error::Auth(ferris_core::AuthError::TokenInvalid(_))
    ));
}

/// `cleanup_states` sweeps the DB too, not just the process-local map.
#[tokio::test]
async fn cleanup_sweeps_expired_db_states() {
    let manager_config = config_with(provider_with_token_url(
        "https://provider.example/token".to_string(),
    ));
    let mut manager = OAuthManager::with_db(manager_config, memory_db().await).unwrap();
    manager
        .state_store
        .put(
            "stale",
            PendingState {
                provider: "test".to_string(),
                pkce_verifier: None,
                expires_at: chrono::Utc::now() - chrono::Duration::seconds(1),
            },
        )
        .await
        .unwrap();

    manager.cleanup_states().await.expect("the sweep succeeds");
    let error = manager
        .take_pending("test", "stale")
        .await
        .expect_err("the swept row is gone");
    assert!(matches!(
        error,
        Error::Auth(ferris_core::AuthError::TokenInvalid(_))
    ));
}

/// Cross-replica: the state minted by one manager is redeemable by another
/// manager over the same database — that is the whole point of the DB store.
#[tokio::test]
async fn a_second_manager_over_the_same_db_sees_the_state() {
    let db = memory_db().await;
    let manager_config = config_with(provider_with_token_url(
        "https://provider.example/token".to_string(),
    ));
    let mut first = OAuthManager::with_db(manager_config.clone(), db.clone()).unwrap();
    let mut second = OAuthManager::with_db(manager_config, db).unwrap();

    let (_, state) = first.authorize("test").await.unwrap();
    second
        .take_pending("test", &state)
        .await
        .expect("the replica that did not mint the state can redeem it");

    let error = first
        .take_pending("test", &state)
        .await
        .expect_err("but only once, across replicas");
    assert!(matches!(
        error,
        Error::Auth(ferris_core::AuthError::TokenInvalid(_))
    ));
}

/// The manager lock must not be held across the token exchange: a slow provider
/// answering after 500 ms cannot stop another request from taking the lock
/// (which is what `list_providers`/`authorize` do in the router).
#[tokio::test]
async fn a_slow_exchange_does_not_hold_the_manager_lock() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(500))
                .set_body_json(serde_json::json!({
                    "access_token": "slow-access",
                    "token_type": "Bearer",
                    "expires_in": 600
                })),
        )
        .mount(&server)
        .await;

    let manager = Arc::new(Mutex::new(
        OAuthManager::new(config_with(provider_with_token_url(format!(
            "{}/token",
            server.uri()
        ))))
        .unwrap(),
    ));
    let state = {
        let mut manager = manager.lock().await;
        let (_, state) = manager.authorize("test").await.unwrap();
        state
    };
    // The lock is released here, exactly as the router does it.
    let (client, verifier) = {
        let mut manager = manager.lock().await;
        manager.take_pending("test", &state).await.unwrap()
    };
    let exchange =
        tokio::spawn(async move { client.exchange_code("code".to_string(), verifier).await });

    let guard = tokio::time::timeout(Duration::from_millis(100), manager.lock())
        .await
        .expect("the manager lock must be free while the exchange is in flight");
    assert_eq!(guard.list_providers(), vec!["test".to_string()]);
    drop(guard);

    let tokens = exchange
        .await
        .expect("the exchange task joins")
        .expect("the exchange succeeds");
    assert_eq!(tokens.access_token, "slow-access");
}

/// `OpenID Connect` is not implemented; a provider that declares it is a config
/// error instead of a login that silently ignores an unvalidated `id_token`.
#[test]
fn an_oidc_provider_is_refused_at_construction() {
    let mut provider = provider_with_token_url("https://provider.example/token".to_string());
    provider.oidc = true;
    let result = OAuthManager::new(config_with(provider));
    assert!(matches!(
        result,
        Err(Error::Config(ferris_core::ConfigError::Invalid { .. }))
    ));
}

/// The store binds the first subject it sees, accepts the same one again, and
/// refuses a different one without overwriting the binding. A second store over
/// the same database sees the same binding (it is a column, not a map).
#[tokio::test]
async fn oauth_subject_binds_once_and_refuses_a_different_identity() {
    let db = memory_db().await;
    let store = crate::UserStore::new(db);
    store.init().await.unwrap();
    let user = store
        .create(
            "oauth-subject@local",
            "OAuth Subject",
            None,
            crate::UserType::SystemUser,
            "",
            &["User".to_string()],
        )
        .await
        .unwrap();

    assert!(store.oauth_subject(&user.id).await.unwrap().is_none());
    assert!(store.bind_oauth_subject(&user.id, "sub-a").await.unwrap());
    assert_eq!(
        store.oauth_subject(&user.id).await.unwrap().as_deref(),
        Some("sub-a")
    );
    // Idempotent for the same identity, refused for another one.
    assert!(store.bind_oauth_subject(&user.id, "sub-a").await.unwrap());
    assert!(!store.bind_oauth_subject(&user.id, "sub-b").await.unwrap());
    assert_eq!(
        store.oauth_subject(&user.id).await.unwrap().as_deref(),
        Some("sub-a"),
        "the existing binding must not be overwritten"
    );

    let other = crate::UserStore::new(store.db().clone());
    assert!(!other.bind_oauth_subject(&user.id, "sub-b").await.unwrap());
    assert!(other.bind_oauth_subject(&user.id, "sub-a").await.unwrap());

    // The administrative setter can clear it; the next login may bind again.
    store.set_oauth_subject(&user.id, None).await.unwrap();
    assert!(store.oauth_subject(&user.id).await.unwrap().is_none());
    assert!(store.bind_oauth_subject(&user.id, "sub-b").await.unwrap());
}

/// AUTH1: `refresh_token` must perform the real RFC 6749 §6 exchange, not
/// return a hardcoded placeholder. The stub asserts the request body is the
/// refresh grant and the reply is echoed back verbatim.
#[tokio::test]
async fn refresh_token_performs_the_real_grant() {
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=old-refresh"))
        .and(body_string_contains("client_id=client"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "new-access",
            "token_type": "Bearer",
            "expires_in": 1200,
            "refresh_token": "new-refresh"
        })))
        .mount(&server)
        .await;

    let client =
        OAuthClient::new(provider_with_token_url(format!("{}/token", server.uri()))).unwrap();

    let tokens = client
        .refresh_token("old-refresh".to_string())
        .await
        .expect("the refresh grant succeeds");
    assert_eq!(tokens.access_token, "new-access");
    assert_eq!(tokens.refresh_token.as_deref(), Some("new-refresh"));
    assert_eq!(tokens.expires_in, 1200);
    assert!(!tokens.access_token.contains("placeholder"));
}

/// AUTH1: a rejected grant is a typed `OAuthTokenRefresh` error, never a
/// fabricated success.
#[tokio::test]
async fn a_rejected_refresh_is_a_typed_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string("invalid_grant"))
        .mount(&server)
        .await;

    let client =
        OAuthClient::new(provider_with_token_url(format!("{}/token", server.uri()))).unwrap();

    let err = client
        .refresh_token("bad".to_string())
        .await
        .expect_err("a 400 must not be a fake success");
    assert!(
        matches!(
            err,
            Error::Auth(ferris_core::AuthError::OAuthTokenRefresh(_))
        ),
        "expected OAuthTokenRefresh, got {err:?}"
    );
}
