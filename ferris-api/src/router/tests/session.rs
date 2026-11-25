//! End-to-end session contract: login, MFA, logout, logout-all, expiry.
//!
//! Every test drives the real router with the real `tower-sessions` +
//! `axum-login` layer; nothing is minted by hand and nothing reads the old
//! custom session store (it no longer exists).

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use ferris_auth::{MfaConfig, MfaManager, TotpAlgorithm, TotpConfig, UserType};
use tower::ServiceExt;

use super::harness::{Login, app, login_as, test_state};

/// Post a login form and return `(status, headers, body)`.
async fn post_login(
    state: &super::harness::TestState,
    username: &str,
    password: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let body = serde_json::json!({ "username": username, "password": password }).to_string();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

fn cookie_pair(headers: &axum::http::HeaderMap, name: &str) -> String {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            let pair = value.split(';').next()?;
            pair.starts_with(&format!("{name}="))
                .then(|| pair.to_string())
        })
        .expect("the login response sets the session cookie")
}

/// Whether the response sets the session cookie at all.
fn sets_session_cookie(headers: &axum::http::HeaderMap, name: &str) -> bool {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| value.starts_with(&format!("{name}=")))
}

fn session_id_of(cookie: &str) -> tower_sessions::session::Id {
    cookie
        .split_once('=')
        .expect("cookie pair")
        .1
        .parse()
        .expect("session id")
}

async fn get_session(state: &super::harness::TestState, cookie: &str) -> (StatusCode, String) {
    let response = app(state)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/auth/session")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn seed_account(state: &super::harness::TestState, email: &str) -> String {
    crate::test_support::install_test_keyring();
    let user = state
        .session
        .user_store
        .create(
            email,
            email,
            None,
            UserType::SystemUser,
            "Harness-Password-123",
            &["System Manager".to_string()],
        )
        .await
        .expect("seed the account");
    user.id
}

async fn logout(state: &super::harness::TestState, login: &Login, path: &str) -> StatusCode {
    app(state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header(header::COOKIE, login.cookie.clone())
                .header(ferris_auth::CSRF_HEADER, login.csrf.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// The happy path: a password login opens a real session, the same cookie
/// authenticates the next request, and `/api/auth/session` reports the identity
/// plus the CSRF token the gate expects.
#[tokio::test]
async fn a_login_opens_a_session_that_authenticates_the_next_request() {
    let state = test_state().await;
    let login = login_as(&state, &["System Manager"]).await;

    let (status, text) = get_session(&state, &login.cookie).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["roles"], serde_json::json!(["System Manager"]));
    assert!(!login.csrf.is_empty());
}

/// A wrong password answers the generic 401 and sets no session cookie.
#[tokio::test]
async fn a_wrong_password_does_not_open_a_session() {
    let state = test_state().await;
    let _ = seed_account(&state, "wrong-password@harness.local").await;
    let (status, headers, text) =
        post_login(&state, "wrong-password@harness.local", "not-the-password").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{text}");
    assert!(
        !headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .any(|v| v.starts_with(&format!("{}=", state.auth.config.cookie_name))),
        "a refused login must not set a session cookie"
    );
}

/// The email-verification gate is off by default: an account that never
/// confirmed its address still logs in, exactly as before the flag existed.
#[tokio::test]
async fn an_unverified_account_logs_in_when_the_policy_is_off() {
    let state = test_state().await;
    let email = "unverified-off@harness.local";
    seed_account(&state, email).await;

    let (status, headers, text) = post_login(&state, email, "Harness-Password-123").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(sets_session_cookie(
        &headers,
        &state.auth.config.cookie_name
    ));
}

/// With the opt-in policy on, a proven password for an unverified account is
/// refused with a `403` and no session. The check runs only after the password
/// matched, so it cannot be used to discover accounts.
#[tokio::test]
async fn an_unverified_account_is_refused_when_the_policy_is_on() {
    let mut state = test_state().await;
    state.session.require_email_verification = true;
    let email = "unverified-on@harness.local";
    seed_account(&state, email).await;

    let (status, headers, text) = post_login(&state, email, "Harness-Password-123").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(
        text.contains("verify your email address before signing in"),
        "{text}"
    );
    assert!(
        !sets_session_cookie(&headers, &state.auth.config.cookie_name),
        "a policy refusal must not open a session"
    );

    // The same wrong password keeps the generic `401`: the policy never runs
    // before the credential is proven.
    let (status, _, text) = post_login(&state, email, "not-the-password").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{text}");
    assert!(!text.contains("verify your email"), "{text}");
}

/// A verified account passes the policy and opens a session.
#[tokio::test]
async fn a_verified_account_logs_in_when_the_policy_is_on() {
    let mut state = test_state().await;
    state.session.require_email_verification = true;
    let email = "verified@harness.local";
    let user_id = seed_account(&state, email).await;
    state
        .session
        .user_store
        .set_email_verified(&user_id, true)
        .await
        .expect("mark the address verified");

    let (status, headers, text) = post_login(&state, email, "Harness-Password-123").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(sets_session_cookie(
        &headers,
        &state.auth.config.cookie_name
    ));
}

/// The policy also stops the MFA challenge: an account that could never
/// complete the login is not handed a pending challenge or a cookie.
#[tokio::test]
async fn an_unverified_account_cannot_start_the_mfa_challenge() {
    let mut state = test_state().await;
    state.session.require_email_verification = true;
    let email = "unverified-mfa@harness.local";
    let user_id = seed_account(&state, email).await;
    let mfa = MfaManager::new(MfaConfig::default());
    let secret = mfa.generate_totp_secret();
    state
        .session
        .user_store
        .set_mfa(&user_id, true, Some(&secret.secret), &[])
        .await
        .unwrap();

    let (status, headers, text) = post_login(&state, email, "Harness-Password-123").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(
        text.contains("verify your email address before signing in"),
        "{text}"
    );
    assert!(!sets_session_cookie(
        &headers,
        &state.auth.config.cookie_name
    ));
}

/// MFA: the password step stores a pending challenge in the session and the
/// code step completes the login. A second use of the same challenge fails.
#[tokio::test]
async fn mfa_is_a_two_step_login_with_a_single_use_challenge() {
    let state = test_state().await;
    let email = "mfa@harness.local";
    let user_id = seed_account(&state, email).await;
    let mfa = MfaManager::new(MfaConfig::default());
    let secret = mfa.generate_totp_secret();
    state
        .session
        .user_store
        .set_mfa(&user_id, true, Some(&secret.secret), &[])
        .await
        .unwrap();

    let (status, headers, text) = post_login(&state, email, "Harness-Password-123").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["mfa_required"], true, "{text}");
    let challenge = json["challenge"].as_str().unwrap().to_string();
    let cookie = cookie_pair(&headers, &state.auth.config.cookie_name);

    // No session yet: the pending cookie is not a login.
    let (status, _) = get_session(&state, &cookie).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let code = mfa
        .generate_totp(&TotpConfig {
            secret: secret.secret.clone(),
            digits: mfa.totp_digits(),
            period: mfa.totp_period(),
            algorithm: TotpAlgorithm::Sha1,
        })
        .unwrap();
    let (status, verify_headers, text) = verify_mfa(&state, &cookie, &challenge, &code).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    // `AuthSession::login` cycles the id (session-fixation mitigation), so the
    // browser now holds a different cookie than the pending step did.
    let cookie = cookie_pair(&verify_headers, &state.auth.config.cookie_name);
    let csrf = verify_headers
        .get(ferris_auth::CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .expect("the completed login exposes the CSRF token")
        .to_string();
    let (status, text) = get_session(&state, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{text}");

    // The challenge is single use: the same cookie cannot replay it.
    let body = serde_json::json!({ "challenge": challenge, "code": code }).to_string();
    let replay = app(&state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/mfa/verify")
                .header(header::COOKIE, &cookie)
                .header(ferris_auth::CSRF_HEADER, &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}

/// Post a `mfa/verify` challenge and return `(status, headers, body)`.
async fn verify_mfa(
    state: &super::harness::TestState,
    cookie: &str,
    challenge: &str,
    code: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let body = serde_json::json!({ "challenge": challenge, "code": code }).to_string();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/mfa/verify")
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Wait until the current TOTP step has at least two seconds left.
///
/// The replay assertion compares two verifications of the same code, and the
/// handler claims the **current** step: if the clock crossed a boundary between
/// them, the second verify would legitimately claim the next step. Aligning to
/// the start of a step keeps the test deterministic without freezing the clock.
fn align_to_fresh_step(period: u64) {
    let period = period.max(1);
    while u64::try_from(chrono::Utc::now().timestamp().max(0)).unwrap_or(0) % period + 2 > period {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// A TOTP code spends its step in the database, so the same code cannot
/// complete a second pending challenge — not here, and not on another replica
/// (AUTH-04). The two challenges are prepared up front so neither verify does
/// password work; the replay is compared against the same step.
#[tokio::test]
async fn a_used_totp_step_is_refused_on_the_next_challenge() {
    let state = test_state().await;
    let email = "totp-replay@harness.local";
    let user_id = seed_account(&state, email).await;
    let mfa = MfaManager::new(MfaConfig::default());
    let secret = mfa.generate_totp_secret();
    state
        .session
        .user_store
        .set_mfa(&user_id, true, Some(&secret.secret), &[])
        .await
        .unwrap();

    let mut challenges = Vec::with_capacity(2);
    for _ in 0..2 {
        let (status, headers, text) = post_login(&state, email, "Harness-Password-123").await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["mfa_required"], true, "{text}");
        challenges.push((
            cookie_pair(&headers, &state.auth.config.cookie_name),
            json["challenge"].as_str().unwrap().to_string(),
        ));
    }

    align_to_fresh_step(state.session.mfa.totp_period());
    let code = mfa
        .generate_totp(&TotpConfig {
            secret: secret.secret.clone(),
            digits: mfa.totp_digits(),
            period: mfa.totp_period(),
            algorithm: TotpAlgorithm::Sha1,
        })
        .unwrap();

    let (status, _, text) = verify_mfa(&state, &challenges[0].0, &challenges[0].1, &code).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let (status, _, text) = verify_mfa(&state, &challenges[1].0, &challenges[1].1, &code).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{text}");
    assert!(text.contains("already used"), "{text}");
}

/// `logout` flushes the record and clears the cookie; the old cookie no longer
/// authenticates.
#[tokio::test]
async fn logout_revokes_the_session_and_clears_the_cookie() {
    let state = test_state().await;
    let login = login_as(&state, &["System Manager"]).await;

    let response = app(&state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/logout")
                .header(header::COOKIE, login.cookie.clone())
                .header(ferris_auth::CSRF_HEADER, login.csrf.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .any(|value| value.contains("Max-Age=0")),
        "logout must expire the cookie"
    );
    let (status, _) = get_session(&state, &login.cookie).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// `logout-all` closes every session of the account, not only the presented
/// one, and the caller's own cookie dies with them.
#[tokio::test]
async fn logout_all_closes_every_session_of_the_account() {
    let state = test_state().await;
    let email = "logout-all@harness.local";
    seed_account(&state, email).await;

    let first = post_login(&state, email, "Harness-Password-123").await;
    let second = post_login(&state, email, "Harness-Password-123").await;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(second.0, StatusCode::OK);
    let first_cookie = cookie_pair(&first.1, &state.auth.config.cookie_name);
    let second_cookie = cookie_pair(&second.1, &state.auth.config.cookie_name);
    let first_csrf = first
        .1
        .get(ferris_auth::CSRF_HEADER)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let status = logout(
        &state,
        &Login {
            cookie: first_cookie.clone(),
            csrf: first_csrf,
        },
        "/api/auth/logout-all",
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    for cookie in [first_cookie, second_cookie] {
        let (status, _) = get_session(&state, &cookie).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "every session of the account must be gone"
        );
    }
}

/// An expired `tower-sessions` record stops authenticating even though the
/// browser still sends the cookie.
#[tokio::test]
async fn an_expired_session_is_not_authenticated() {
    use tower_sessions::SessionStore;

    let state = test_state().await;
    let login = login_as(&state, &["System Manager"]).await;
    let id = session_id_of(&login.cookie);
    let mut record = state
        .store
        .load(&id)
        .await
        .expect("session lookup")
        .expect("the login wrote a record");
    record.expiry_date = time::OffsetDateTime::now_utc() - time::Duration::minutes(1);
    state.store.save(&record).await.expect("expire the record");

    let (status, _) = get_session(&state, &login.cookie).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
