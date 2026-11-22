//! The OAuth flow's contract, over real HTTP.
//!
//! Every test drives the **router** (not the handlers) with `tower::oneshot`, so
//! what is asserted is what a browser sees: status, `Location`, and the
//! `Set-Cookie`/`X-CSRF-Token` headers. The provider's two endpoints (token
//! exchange and userinfo) are `wiremock` mocks — no request leaves the process.

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use ferris_auth::oauth::{NEXT_COOKIE, STATE_COOKIE};
use ferris_auth::{
    OAuthConfig, OAuthManager, OAuthProviderConfig, PkceMethod, SessionConfig, SessionManager,
    StateStoreConfig, UserType,
};
use ferris_core::Value;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::routes::{DEFAULT_NEXT, error_response, safe_next};
use super::{OAuthState, oauth_router_with_state};
use crate::SessionHandlers;
use crate::session::cookie_value;

const EMAIL: &str = "ada@example.com";

/// One provider pointed at `server`, PKCE on (the flow forces S256).
fn provider(server: &MockServer) -> OAuthProviderConfig {
    OAuthProviderConfig {
        name: "github".to_string(),
        client_id: "client-id".to_string(),
        client_secret: "client-secret".to_string(),
        auth_url: format!("{}/authorize", server.uri()),
        token_url: format!("{}/token", server.uri()),
        user_info_url: Some(format!("{}/userinfo", server.uri())),
        device_auth_url: None,
        scopes: vec!["read:user".to_string()],
        pkce_enabled: true,
        pkce_method: PkceMethod::S256,
        redirect_uris: vec!["https://app.example.com/api/oauth/github/callback".to_string()],
    }
}

fn config(server: &MockServer) -> OAuthConfig {
    OAuthConfig {
        providers: vec![provider(server)],
        state_store: StateStoreConfig {
            enabled: true,
            ttl_seconds: 600,
        },
    }
}

/// The token endpoint and the userinfo endpoint of a healthy provider.
async fn provider_mocks(server: &MockServer, email: &str) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "provider-access-token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": "provider-refresh-token"
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "email": email, "login": "ada" })),
        )
        .mount(server)
        .await;
}

/// A login gate on an in-memory database with one usable account.
async fn handlers() -> SessionHandlers {
    let config = SessionConfig {
        // The test client speaks plain HTTP; a `Secure` cookie would never come
        // back, and these assertions are about the flow, not about TLS.
        secure_cookies: false,
        ..SessionConfig::default()
    };
    let handlers = SessionHandlers::new(
        config.clone(),
        SessionManager::new(config),
        ferris_db::memory_db().await.expect("in-memory db"),
    );
    handlers.user_store.init().await.expect("user table");
    // An empty password means "no password login": the point of the account is
    // the provider-proven email, and hashing one would cost seconds of Argon2
    // for nothing.
    handlers
        .user_store
        .create(
            EMAIL,
            "Ada Lovelace",
            Some("ada"),
            UserType::SystemUser,
            "",
            &["System Manager".to_string()],
        )
        .await
        .expect("seed the oauth account");
    handlers
}

/// The router under test plus the state it shares, so a test can prove the login
/// really opened a session on the same `SessionManager` the auth middleware uses.
async fn app(server: &MockServer) -> (Router, OAuthState) {
    let state = OAuthState::new(
        OAuthManager::new(config(server)).expect("oauth manager"),
        handlers().await,
    );
    let router: Router = oauth_router_with_state().with_state(state.clone());
    (router, state)
}

fn get(uri: &str, cookies: &[(&str, String)]) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if !cookies.is_empty() {
        let jar = cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        builder = builder.header(header::COOKIE, jar);
    }
    builder.body(Body::empty()).expect("request")
}

/// Every `Set-Cookie` value in the response.
fn cookies(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect()
}

fn cookie_pair(set_cookies: &[String], name: &str) -> Option<String> {
    set_cookies.iter().find_map(|raw| {
        let (pair, _) = raw.split_once(';').unwrap_or((raw, ""));
        let (cookie_name, value) = pair.split_once('=')?;
        (cookie_name.trim() == name).then(|| value.trim().to_string())
    })
}

fn location(headers: &HeaderMap) -> String {
    headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("a redirect")
        .to_string()
}

/// The `state` query parameter of an authorization URL (the provider echoes it
/// back verbatim, so it is what the browser must present).
fn state_of(authorize_url: &str) -> String {
    let (_, query) = authorize_url.split_once('?').expect("authorize URL query");
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("state="))
        .expect("authorize URL carries the state")
        .to_string()
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("json body")
}

/// Start the flow and return `(state, next)` as the browser would hold them.
async fn start(router: &Router, next: Option<&str>) -> (String, String) {
    let uri = next.map_or_else(
        || "/api/oauth/github/login".to_string(),
        |next| format!("/api/oauth/github/login?next={next}"),
    );
    let response = router.clone().oneshot(get(&uri, &[])).await.expect("login");
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    let set_cookies = cookies(response.headers());
    let state = cookie_pair(&set_cookies, STATE_COOKIE).expect("state cookie");
    assert_eq!(
        state,
        state_of(&location(response.headers())),
        "the cookie binds the same state the provider will echo"
    );
    let next = cookie_pair(&set_cookies, NEXT_COOKIE).expect("next cookie");
    (state, next)
}

/// The happy path: the login binds a state to the browser, the callback checks
/// it, opens a real session and lands on `next`.
#[tokio::test]
async fn the_callback_opens_a_session_and_redirects_to_next() {
    let server = MockServer::start().await;
    provider_mocks(&server, EMAIL).await;
    let (router, state) = app(&server).await;

    let (oauth_state, next) = start(&router, Some("/desk/module/doctype/row")).await;
    assert_eq!(next, "/desk/module/doctype/row");

    let callback = router
        .clone()
        .oneshot(get(
            &format!("/api/oauth/github/callback?code=the-code&state={oauth_state}"),
            &[(STATE_COOKIE, oauth_state), (NEXT_COOKIE, next.clone())],
        ))
        .await
        .expect("callback");
    let status = callback.status();
    if status != StatusCode::SEE_OTHER {
        let body = body_json(callback).await;
        panic!("the callback must open a session, got {status}: {body:?}");
    }
    assert_eq!(location(callback.headers()), next, "lands on `next`");

    let set_cookies = cookies(callback.headers());
    let session_id =
        cookie_pair(&set_cookies, "ferris_session").expect("the callback sets the session cookie");
    let csrf = callback
        .headers()
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .expect("the callback exposes the session CSRF token")
        .to_string();
    // The flow cookies are deleted, so the browser cannot replay them.
    for name in [STATE_COOKIE, NEXT_COOKIE] {
        assert!(
            set_cookies
                .iter()
                .any(|raw| raw.starts_with(&format!("{name}=")) && raw.contains("Max-Age=0")),
            "`{name}` must be cleared: {set_cookies:?}"
        );
    }
    // The provider's tokens never reach the browser.
    let leaked =
        serde_json::json!({ "cookies": set_cookies, "location": location(callback.headers()) })
            .to_string();
    assert!(
        !leaked.contains("provider-access-token") && !leaked.contains("provider-refresh-token"),
        "provider tokens must not travel to the client: {leaked}"
    );

    // The session really exists, belongs to the resolved account, and its CSRF
    // token is the one the callback handed out.
    let stored = state
        .session
        .session_manager
        .get_session(&session_id)
        .await
        .expect("session lookup")
        .expect("the oauth login created a session");
    let user = state
        .session
        .user_store
        .find_by_id(&stored.user_id)
        .await
        .expect("user lookup")
        .expect("the session belongs to a real user");
    assert_eq!(user.email, EMAIL);
    assert_eq!(stored.csrf_token, csrf);
    assert!(
        stored.ip_address.is_some(),
        "the session records the client ip"
    );
}

/// Without the browser's state cookie the callback is refused **before** the
/// code is exchanged: no session, no tokens, no request to the provider.
#[tokio::test]
async fn the_callback_without_the_state_cookie_is_refused() {
    let server = MockServer::start().await;
    provider_mocks(&server, EMAIL).await;
    let (router, _state) = app(&server).await;

    let (oauth_state, _) = start(&router, None).await;
    let callback = router
        .oneshot(get(
            &format!("/api/oauth/github/callback?code=the-code&state={oauth_state}"),
            &[],
        ))
        .await
        .expect("callback");
    assert_eq!(callback.status(), StatusCode::FORBIDDEN);
    assert!(
        cookies(callback.headers())
            .iter()
            .all(|raw| !raw.starts_with("ferris_session=")),
        "a refused callback must not set a session cookie"
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("recorded requests")
            .len(),
        0,
        "the authorization code must not be exchanged"
    );
}

/// A state cookie that does not match the `state` in the URL (a pair minted for
/// another browser) is refused.
#[tokio::test]
async fn a_state_from_another_browser_is_refused() {
    let server = MockServer::start().await;
    provider_mocks(&server, EMAIL).await;
    let (router, _state) = app(&server).await;

    let (first_state, _) = start(&router, None).await;
    let (second_state, _) = start(&router, None).await;
    assert_ne!(first_state, second_state);

    let callback = router
        .oneshot(get(
            &format!("/api/oauth/github/callback?code=the-code&state={second_state}"),
            &[(STATE_COOKIE, first_state)],
        ))
        .await
        .expect("callback");
    assert_eq!(callback.status(), StatusCode::FORBIDDEN);
}

/// The state is single use: replaying the exact callback (cookie included) after
/// a successful login fails, because the manager consumed it.
#[tokio::test]
async fn a_replayed_callback_is_refused() {
    let server = MockServer::start().await;
    provider_mocks(&server, EMAIL).await;
    let (router, _state) = app(&server).await;

    let (oauth_state, _) = start(&router, None).await;
    let url = format!("/api/oauth/github/callback?code=the-code&state={oauth_state}");

    let first = router
        .clone()
        .oneshot(get(&url, &[(STATE_COOKIE, oauth_state.clone())]))
        .await
        .expect("callback");
    assert_eq!(first.status(), StatusCode::SEE_OTHER);

    let replay = router
        .oneshot(get(&url, &[(STATE_COOKIE, oauth_state)]))
        .await
        .expect("callback");
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}

/// An identity the provider proves but the site has no account for is refused
/// instead of being provisioned on the fly.
#[tokio::test]
async fn an_unknown_email_is_refused() {
    let server = MockServer::start().await;
    provider_mocks(&server, "stranger@example.com").await;
    let (router, _state) = app(&server).await;

    let (oauth_state, _) = start(&router, None).await;
    let callback = router
        .oneshot(get(
            &format!("/api/oauth/github/callback?code=the-code&state={oauth_state}"),
            &[(STATE_COOKIE, oauth_state)],
        ))
        .await
        .expect("callback");
    assert_eq!(callback.status(), StatusCode::FORBIDDEN);
}

/// A state minted for one provider cannot be redeemed on another provider's
/// callback path: the manager checks the provider the state was minted for, not
/// the one in the (attacker-controlled) path.
#[tokio::test]
async fn a_state_cannot_be_used_with_another_provider() {
    let server = MockServer::start().await;
    provider_mocks(&server, EMAIL).await;
    let (router, _state) = app(&server).await;

    let (oauth_state, _) = start(&router, None).await;
    let callback = router
        .oneshot(get(
            &format!("/api/oauth/gitlab/callback?code=the-code&state={oauth_state}"),
            &[(STATE_COOKIE, oauth_state)],
        ))
        .await
        .expect("callback");
    // The state is invalid *for this provider*: an auth error, never a token
    // exchange through the wrong client.
    assert_eq!(callback.status(), StatusCode::UNAUTHORIZED);
    let body = body_json(callback).await.to_string();
    assert!(body.contains("issued for provider"), "{body}");
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("recorded requests")
            .len(),
        0,
        "the authorization code must not be exchanged"
    );
}

/// `next` is a local path or nothing: an absolute URL, a protocol-relative URL
/// and control characters all fall back to `/desk`.
#[test]
fn next_is_only_ever_a_local_path() {
    assert_eq!(
        safe_next(Some("/desk/module/doctype")),
        "/desk/module/doctype"
    );
    assert_eq!(safe_next(Some("  /desk  ")), "/desk");
    assert_eq!(safe_next(None), DEFAULT_NEXT);
    assert_eq!(safe_next(Some("")), DEFAULT_NEXT);
    for hostile in [
        "//evil.example/steal",
        "https://evil.example",
        "evil.example",
        "/desk\r\nSet-Cookie: x=1",
        "/desk\\..\\evil",
    ] {
        assert_eq!(safe_next(Some(hostile)), DEFAULT_NEXT, "{hostile}");
    }
}

/// Cookie parsing is exact: a cookie whose *name* merely ends with the one asked
/// for must not match, and an empty value reads as absent.
#[test]
fn cookie_lookup_matches_the_exact_name() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        "xferris_oauth_state=wrong; ferris_oauth_next=/desk; empty="
            .parse()
            .expect("cookie header"),
    );
    assert_eq!(cookie_value(&headers, STATE_COOKIE), None);
    assert_eq!(
        cookie_value(&headers, NEXT_COOKIE).as_deref(),
        Some("/desk")
    );
    assert_eq!(cookie_value(&headers, "empty"), None);
}

/// A provider/storage failure is a 5xx, and its text stays in the log: the
/// callback runs against an unauthenticated browser and the message can name a
/// table or an internal path.
#[tokio::test]
async fn a_server_error_does_not_leak_the_internal_reason() {
    let response = error_response(ferris_core::Error::Database(
        ferris_core::error::DatabaseError::Query("insert into tab_oauth_state failed".to_string()),
    ));
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(!text.contains("tab_oauth_state"), "{text}");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["error"], "internal error");
}
