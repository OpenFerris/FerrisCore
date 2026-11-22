use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use ferris_core::Value;
use tower::ServiceExt;

use super::harness::{TestState, app, request, test_state};

/// A live session cookie: (session id, csrf token). `roles` become the
/// session's stored `roles` data, the way login writes them, so
/// cookie-authenticated handlers resolve the caller with real roles.
async fn cookie_session(state: &TestState, roles: &[&str]) -> (String, String) {
    let session = state
        .auth
        .session_manager
        .create_session("user-1".to_string(), None, None)
        .await
        .unwrap();
    if !roles.is_empty() {
        state
            .auth
            .session_manager
            .update_session(
                &session.id,
                HashMap::from([(
                    "roles".to_string(),
                    Value::Array(roles.iter().map(|r| Value::String(r.to_string())).collect()),
                )]),
            )
            .await
            .unwrap();
    }
    (session.id.clone(), session.csrf_token.clone())
}

/// A request authenticated by session cookie only (no bearer), with an
/// optional `X-CSRF-Token` header — the shape the Desk's web client
/// sends after login.
async fn cookie_request(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Option<String>,
    state: &TestState,
    session_id: &str,
    csrf: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri).header(
        header::COOKIE,
        format!("{}={}", state.auth.config.cookie_name, session_id),
    );
    if let Some(token) = csrf {
        builder = builder.header(ferris_auth::CSRF_HEADER, token);
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let response = app
        .oneshot(builder.body(Body::from(body.unwrap_or_default())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
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
    let (sid, token) = cookie_session(&state, &["System Manager"]).await;
    let body = role_permission_body();

    // Missing header: the gate answers, the handler never runs.
    let (status, text) = cookie_request(
        app(&state),
        Method::POST,
        "/api/authz/role-permissions",
        Some(body.clone()),
        &state,
        &sid,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(
        text.contains("CSRF token missing or invalid"),
        "the 403 comes from the CSRF gate: {text}"
    );

    // Wrong token: same wall.
    let (status, text) = cookie_request(
        app(&state),
        Method::POST,
        "/api/authz/role-permissions",
        Some(body.clone()),
        &state,
        &sid,
        Some("not-the-token"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");

    // The session's own token: the handler runs and persists the row.
    let (status, text) = cookie_request(
        app(&state),
        Method::POST,
        "/api/authz/role-permissions",
        Some(body.clone()),
        &state,
        &sid,
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let rows: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["doctype"], "ToDo");
}

/// A20: safe methods carry no side effects, so a cookie GET needs no
/// token — the gate steps aside and the handler answers.
#[tokio::test]
async fn cookie_gets_skip_the_csrf_gate() {
    let state = test_state().await;
    let (sid, _token) = cookie_session(&state, &[]).await;

    let (status, text) = cookie_request(
        app(&state),
        Method::GET,
        "/api/openapi.json",
        None,
        &state,
        &sid,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");

    // And a cookie PUT without the token is still refused: only GET /
    // HEAD / OPTIONS are exempt, not every read-looking verb.
    let (status, text) = cookie_request(
        app(&state),
        Method::PUT,
        "/api/resource/ToDo/csrf-case",
        Some(create_body()),
        &state,
        &sid,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
}

/// A20: bearer-JWT callers are exempt — a cross-site attacker cannot make
/// a victim's browser attach the header without already stealing the
/// token. (Every other test in this module runs on bearer alone and
/// depends on this exemption.)
#[tokio::test]
async fn bearer_mutations_are_exempt_from_csrf() {
    let state = test_state().await;
    let (status, body) = request(
        app(&state),
        Method::POST,
        "/api/authz/role-permissions",
        Some(role_permission_body()),
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
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
