use std::sync::Arc;

use axum::body::Body;
use axum::extract::FromRef;
use axum::http::{Method, Request, StatusCode, header};
use axum::middleware;
use ferris_auth::{AuthState, JwtConfig, JwtManager, SessionConfig, SessionManager};
use ferris_core::FeatureFlags;
use tower::ServiceExt;

use super::super::ApiState;
use crate::module::Module;

/// The smallest state that satisfies the API router's bounds and carries
/// the auth state the `require_auth` middleware needs.
#[derive(Clone)]
pub(super) struct TestState {
    pub(super) api: Arc<ApiState>,
    pub(super) auth: AuthState,
}

impl FromRef<TestState> for Arc<ApiState> {
    fn from_ref(state: &TestState) -> Self {
        state.api.clone()
    }
}

impl FromRef<TestState> for AuthState {
    fn from_ref(state: &TestState) -> Self {
        state.auth.clone()
    }
}

pub(super) async fn test_state() -> TestState {
    test_state_with(FeatureFlags::default()).await
}

pub(super) async fn test_state_with_openapi(
    features: FeatureFlags,
    fragments: Vec<crate::module::OpenApiFragment>,
) -> TestState {
    let mut state = test_state_with(features).await;
    state.api = Arc::new((*state.api).clone().with_openapi_fragments(fragments));
    state
}

pub(super) async fn test_state_with(features: FeatureFlags) -> TestState {
    let pool = ferris_db::memory_db().await.unwrap();
    let auth = AuthState::new(
        JwtManager::new(JwtConfig::new("test-secret-that-is-at-least-32-chars-long").unwrap())
            .unwrap(),
        SessionManager::new(SessionConfig::default()),
        SessionConfig::default(),
    );
    TestState {
        api: Arc::new(ApiState::with_policy_path(pool, None).with_features(features)),
        auth,
    }
}

fn bearer(state: &TestState, roles: &[&str]) -> String {
    let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
    format!(
        "Bearer {}",
        state
            .auth
            .jwt_manager
            .generate_tokens("user-1", "user@example.com", &roles, None, "")
            .unwrap()
            .access_token
    )
}

pub(super) fn app(state: &TestState) -> axum::Router {
    app_with_modules(state, &[])
}

/// Like [`app`], plus the native modules the composition root registered.
pub(super) fn app_with_modules(state: &TestState, modules: &[Arc<dyn Module>]) -> axum::Router {
    // Mirror `build_router`: keep `Router<TestState>` while merging and
    // layering, then resolve the state so the router is servable.
    let router: axum::Router<TestState> = axum::Router::new().merge(
        super::super::api_router_with_modules(state.clone(), modules).layer(
            middleware::from_fn_with_state(
                state.auth.clone(),
                ferris_auth::middleware::require_auth,
            ),
        ),
    );
    router.with_state(state.clone())
}

pub(super) async fn request(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Option<String>,
    state: &TestState,
    roles: &[&str],
) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, bearer(state, roles));
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

/// Like [`request`], but takes extra request headers and returns the
/// response headers too, so a caller can assert on `ETag` (the
/// desired-state tests need both).
pub(super) async fn request_raw(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Option<String>,
    headers: &[(&str, &str)],
    state: &TestState,
    roles: &[&str],
) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, bearer(state, roles));
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = app
        .oneshot(builder.body(Body::from(body.unwrap_or_default())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let response_headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        response_headers,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}
