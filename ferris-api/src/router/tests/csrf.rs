use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use tower::ServiceExt;

use super::harness::{Login, app, login_as, request, test_state};

/// A request authenticated by the session cookie only, with an optional
/// `X-CSRF-Token` header — the shape the Desk's web client sends after login.
fn cookie_request(
    method: Method,
    uri: &str,
    body: Option<String>,
    login: &Login,
    csrf: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, login.cookie.clone());
    if let Some(token) = csrf {
        builder = builder.header(ferris_auth::CSRF_HEADER, token);
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    builder.body(Body::from(body.unwrap_or_default())).unwrap()
}

fn create_body() -> String {
    serde_json::json!({ "name": "csrf-case", "value": 1 }).to_string()
}

fn role_permission_body() -> String {
    serde_json::json!({ "doctype": "ToDo", "role": "Sales", "read": true }).to_string()
}

/// A20: a cookie-authenticated mutation without the session's CSRF token
/// is stopped at the gate (403, the gate's own message — not a handler's
/// answer); the same request with the session's own token reaches the
/// handler and performs the mutation.
#[tokio::test]
async fn cookie_mutations_carry_the_session_csrf_token() {
    let state = test_state().await;
    let login = login_as(&state, &["System Manager"]).await;
    let body = role_permission_body();

    // Missing header: the gate answers, the handler never runs.
    let response = app(&state)
        .oneshot(cookie_request(
            Method::POST,
            "/api/authz/role-permissions",
            Some(body.clone()),
            &login,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let text = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        text.contains("CSRF token missing or invalid"),
        "the 403 comes from the CSRF gate: {text}"
    );

    // Wrong token: same wall.
    let response = app(&state)
        .oneshot(cookie_request(
            Method::POST,
            "/api/authz/role-permissions",
            Some(body.clone()),
            &login,
            Some("not-the-token"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // The session's own token: the handler runs and persists the row.
    let response = app(&state)
        .oneshot(cookie_request(
            Method::POST,
            "/api/authz/role-permissions",
            Some(body),
            &login,
            Some(&login.csrf),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["doctype"], "ToDo");
}

/// A20: safe methods carry no side effects, so a cookie GET needs no
/// token — the gate steps aside and the handler answers.
#[tokio::test]
async fn cookie_gets_skip_the_csrf_gate() {
    let state = test_state().await;
    let login = login_as(&state, &[]).await;

    let response = app(&state)
        .oneshot(cookie_request(
            Method::GET,
            "/api/openapi.json",
            None,
            &login,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // And a cookie PUT without the token is still refused: only GET /
    // HEAD / OPTIONS are exempt, not every read-looking verb.
    let response = app(&state)
        .oneshot(cookie_request(
            Method::PUT,
            "/api/resource/ToDo/csrf-case",
            Some(create_body()),
            &login,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// AUTH-01: a well-formed `Authorization: token <key>:<secret>` is an explicit
/// header, not ambient browser state, so it authenticates and is exempt from
/// the CSRF gate.
#[tokio::test]
async fn api_key_mutations_authenticate_and_skip_csrf() {
    use std::sync::Arc;

    let mut state = test_state().await;
    // The key's owner must exist so the validator can resolve the role.
    let owner = state
        .session
        .user_store
        .create(
            "apikey@harness.local",
            "API Key Owner",
            None,
            ferris_auth::UserType::SystemUser,
            "Harness-Password-123",
            &["System Manager".to_string()],
        )
        .await
        .unwrap();
    let manager = ferris_auth::ApiKeyManager::new();
    let (record, secret) = manager.create(&owner.id, None, None).await.unwrap();
    state.auth = state.auth.clone().with_api_keys(Arc::new(manager));

    let response = app(&state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/authz/role-permissions")
                .header(
                    header::AUTHORIZATION,
                    format!("token {}:{}", record.key, secret),
                )
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(role_permission_body()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let text = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(status, StatusCode::OK, "{text}");
}

/// Layering: the CSRF gate sits *inside* `require_auth`, so a request
/// with no credentials at all is answered 401 by the auth layer and
/// never reaches the gate's 403.
#[tokio::test]
async fn unauthenticated_mutations_get_401_not_the_csrf_403() {
    let state = test_state().await;
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/resource/ToDo")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(create_body()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// A Desk/SSR request with no session is refused by the gate, not rendered as a
/// logged-in page. The Desk's own guard turns this into a redirect, but the API
/// contract underneath is a `401`.
#[tokio::test]
async fn a_request_without_a_session_is_unauthenticated() {
    let state = test_state().await;
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/auth/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// The harness's `request` helper still works for a plain authenticated call,
/// proving the cookie+CSRF pair it captured is accepted by the real gate.
#[tokio::test]
async fn a_captured_login_authenticates_the_api() {
    let state = test_state().await;
    let (status, text) = request(
        app(&state),
        Method::GET,
        "/api/openapi.json",
        None,
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
}
