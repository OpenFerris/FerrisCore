use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::FromRef;
use axum::http::{Method, Request, StatusCode, header};
use axum::middleware;
use ferris_auth::login::{Backend, DbSessionStore, SessionConfig};
use ferris_auth::{AuthState, UserStore};

use crate::SessionHandlers;
use ferris_core::FeatureFlags;
use tower::ServiceExt;

use super::super::ApiState;
use crate::module::Module;

/// A live login captured from the real login endpoint: the `Cookie` header
/// value and the `X-CSRF-Token` the Desk echoes back.
#[derive(Clone)]
pub(super) struct Login {
    pub(super) cookie: String,
    pub(super) csrf: String,
}

/// The smallest state that satisfies the API router's bounds and carries the
/// one session stack the `require_auth` middleware reads.
#[derive(Clone)]
pub(super) struct TestState {
    pub(super) api: Arc<ApiState>,
    pub(super) auth: AuthState,
    pub(super) session: SessionHandlers,
    pub(super) backend: Backend,
    pub(super) store: DbSessionStore,
    /// One login per role set, created through `POST /api/auth/login` on the
    /// first request. Logging in on every request would be correct but would
    /// multiply Argon2 work; sharing the cookie keeps the harness fast.
    pub(super) logins: Arc<Mutex<HashMap<String, Login>>>,
}

impl FromRef<TestState> for Arc<ApiState> {
    fn from_ref(state: &TestState) -> Self {
        state.api.clone()
    }
}

impl FromRef<TestState> for AuthState {
    fn from_ref(state: &TestState) -> AuthState {
        state.auth.clone()
    }
}

impl FromRef<TestState> for SessionHandlers {
    fn from_ref(state: &TestState) -> SessionHandlers {
        state.session.clone()
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
    let store = DbSessionStore::new(pool.clone());
    store.ensure_table().await.unwrap();
    let backend = Backend::new(pool.clone());
    backend.init().await.unwrap();
    let config = SessionConfig::default();
    let auth = AuthState::new(config.clone())
        .with_session_store(store.clone())
        .with_user_validator(Arc::new(UserStore::new(pool.clone())));
    let session = SessionHandlers::new(config, store.clone(), pool.clone());
    TestState {
        api: Arc::new(ApiState::with_policy_path(pool, None).with_features(features)),
        auth,
        session,
        backend,
        store,
        logins: Arc::new(Mutex::new(HashMap::new())),
    }
}

/// Build the router under test with the same layers the composition root
/// mounts: the API router, its auth/CSRF gates, the public auth endpoints and
/// the `tower-sessions` + `axum-login` layer **outside** all of them.
pub(super) fn app(state: &TestState) -> axum::Router {
    app_with_modules(state, &[])
}

/// Like [`app`], plus the native modules the composition root registered.
pub(super) fn app_with_modules(state: &TestState, modules: &[Arc<dyn Module>]) -> axum::Router {
    let session_layer = ferris_auth::login::auth_layer(
        &state.auth.config,
        state.store.clone(),
        state.backend.clone(),
    );
    // Mirror `build_router`: keep `Router<TestState>` while merging and
    // layering, then resolve the state so the router is servable.
    // The API router carries `require_auth`; the public auth endpoints do not
    // (they are the way in). Applying the gate to the whole merged router — as
    // an earlier version of this harness did — makes login answer 401.
    let guarded_api = super::super::api_router_with_modules(state.clone(), modules).layer(
        middleware::from_fn_with_state(state.auth.clone(), ferris_auth::middleware::require_auth),
    );
    let router: axum::Router<TestState> = axum::Router::<TestState>::new()
        .route(
            "/api/auth/login",
            axum::routing::post(crate::session::login),
        )
        .route(
            "/api/auth/session",
            axum::routing::get(crate::session::session),
        )
        .route(
            "/api/auth/mfa/verify",
            axum::routing::post(crate::session::verify_mfa),
        )
        .merge(
            axum::Router::new()
                .route(
                    "/api/auth/logout",
                    axum::routing::post(crate::session::logout),
                )
                .route(
                    "/api/auth/logout-all",
                    axum::routing::post(crate::session::logout_all),
                )
                .route_layer(middleware::from_fn_with_state(
                    state.auth.clone(),
                    ferris_auth::middleware::require_csrf,
                )),
        )
        .merge(guarded_api)
        .layer(session_layer);
    router.with_state(state.clone())
}

/// Log `roles` in through `POST /api/auth/login` and return the cookie + CSRF
/// token every guarded request needs.
pub(super) async fn login_as(state: &TestState, roles: &[&str]) -> Login {
    let key = roles.join(",");
    if let Some(login) = state.logins.lock().unwrap().get(&key).cloned() {
        return login;
    }
    let login = create_login(state, roles).await;
    state.logins.lock().unwrap().insert(key, login.clone());
    login
}

async fn create_login(state: &TestState, roles: &[&str]) -> Login {
    let slug = if roles.is_empty() {
        "guest".to_string()
    } else {
        roles.join("-").to_lowercase().replace(' ', "_")
    };
    let email = format!("{slug}@harness.local");
    let password = "Harness-Password-123";
    // The account is created through the store so the real backend verifies the
    // real Argon2 hash, exactly like a production login.
    if state
        .session
        .user_store
        .find_by_login(&email)
        .await
        .unwrap()
        .is_none()
    {
        let roles: Vec<String> = roles.iter().map(|role| (*role).to_string()).collect();
        state
            .session
            .user_store
            .create(
                &email,
                &email,
                None,
                ferris_auth::UserType::SystemUser,
                password,
                &roles,
            )
            .await
            .unwrap();
    }

    let body = serde_json::json!({ "username": email, "password": password }).to_string();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("the login request builds"),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "harness login failed: {status} {}",
        String::from_utf8_lossy(&bytes)
    );
    let cookie_name = &state.auth.config.cookie_name;
    let cookie = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            let pair = value.split(';').next()?;
            pair.starts_with(&format!("{cookie_name}="))
                .then(|| pair.to_string())
        })
        .expect("the login response sets the session cookie");
    let csrf = headers
        .get(ferris_auth::CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .expect("the login response exposes the CSRF token")
        .to_string();
    Login { cookie, csrf }
}

pub(super) async fn request(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Option<String>,
    state: &TestState,
    roles: &[&str],
) -> (StatusCode, String) {
    let login = login_as(state, roles).await;
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, login.cookie)
        .header(ferris_auth::CSRF_HEADER, login.csrf);
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
    let login = login_as(state, roles).await;
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, login.cookie)
        .header(ferris_auth::CSRF_HEADER, login.csrf);
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
