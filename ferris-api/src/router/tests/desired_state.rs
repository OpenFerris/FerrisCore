use axum::http::{Method, StatusCode, header};

use super::harness::{app, request_raw, test_state};

/// The ambient engine user the desired-state tests run as. The production
/// router binds this from the authenticated session (`bind_ambient_user`);
/// this harness has to set it explicitly, because the handlers themselves
/// never extract `AuthenticatedUser`.
fn engine_user(id: &str, roles: &[&str]) -> ferris_auth::User {
    let mut user = ferris_auth::User::system_user(id.to_string(), "User 1".to_string());
    user.id = id.to_string();
    user.roles = roles.iter().copied().map(str::to_string).collect();
    user
}

/// R-FW-1b: the publish/pull half of the desired-state API. A control plane
/// publishes a versioned state; an agent pulls it and can poll
/// conditionally with `If-None-Match` so an unchanged state costs a `304`.
#[tokio::test]
async fn desired_state_publishes_and_pulls_with_a_conditional_get() {
    crate::doctypes::register_doctypes();
    let state = test_state().await;
    ferris_document::with_user(Some(engine_user("user-1", &["System Manager"])), async {
        // Nothing published yet: a pull is a 404, not an empty 200.
        let (status, _, _) = request_raw(
            app(&state),
            Method::GET,
            "/api/desired-state/edge-1",
            None,
            &[],
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Publish: the first write creates the generation and answers 201
        // with its `ETag`.
        let (status, headers, body) = request_raw(
            app(&state),
            Method::PUT,
            "/api/desired-state/edge-1",
            Some(serde_json::json!({ "payload": { "speed": 10 } }).to_string()),
            &[],
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(headers.get(header::ETAG).unwrap(), "\"1\"");
        let published: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(published["version"], 1);
        assert_eq!(published["payload"]["speed"], 10);

        // Pull: the same generation comes back with its validator.
        let (status, headers, body) = request_raw(
            app(&state),
            Method::GET,
            "/api/desired-state/edge-1",
            None,
            &[],
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(headers.get(header::ETAG).unwrap(), "\"1\"");
        let pulled: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(pulled["payload"]["speed"], 10);

        // Conditional pull holding the current version: 304, no body.
        let (status, headers, _) = request_raw(
            app(&state),
            Method::GET,
            "/api/desired-state/edge-1",
            None,
            &[("if-none-match", "\"1\"")],
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);
        assert_eq!(headers.get(header::ETAG).unwrap(), "\"1\"");
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "no-cache",
            "a conditional poll revalidates every time"
        );

        // A stale validator still gets the current body.
        let (status, _, body) = request_raw(
            app(&state),
            Method::GET,
            "/api/desired-state/edge-1",
            None,
            &[("if-none-match", "\"0\"")],
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // The list route sees the published entity.
        let (status, _, body) = request_raw(
            app(&state),
            Method::GET,
            "/api/desired-state",
            None,
            &[],
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let listed: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["entity"], "edge-1");
    })
    .await;
}

/// R-FW-1b: `expected_version` turns a publish into a compare-and-set, and
/// a stale writer is told the current generation through a `409` instead of
/// clobbering a newer one.
#[tokio::test]
async fn desired_state_publish_honours_expected_version() {
    crate::doctypes::register_doctypes();
    let state = test_state().await;
    ferris_document::with_user(Some(engine_user("user-1", &["System Manager"])), async {
        // Expecting the absent generation (0) accepts the first publish.
        let (status, _, body) = request_raw(
            app(&state),
            Method::PUT,
            "/api/desired-state/edge-2",
            Some(serde_json::json!({ "payload": "v1", "expected_version": 0 }).to_string()),
            &[],
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        // A second writer that still believes it is generation 0 is refused.
        let (status, _, body) = request_raw(
            app(&state),
            Method::PUT,
            "/api/desired-state/edge-2",
            Some(serde_json::json!({ "payload": "stale", "expected_version": 0 }).to_string()),
            &[],
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        let conflict: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(conflict["error"], "version_conflict");
        assert_eq!(conflict["current_version"], 1);

        // The writer that caught up wins.
        let (status, headers, _) = request_raw(
            app(&state),
            Method::PUT,
            "/api/desired-state/edge-2",
            Some(serde_json::json!({ "payload": "v2", "expected_version": 1 }).to_string()),
            &[],
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get(header::ETAG).unwrap(), "\"2\"");
    })
    .await;
}

/// R-FW-1b: a signed-in agent that is not a `System Manager` may pull but
/// may not publish — the write half is gated by the engine policy, while
/// `All` holds the read grant.
#[tokio::test]
async fn desired_state_pulls_are_open_but_publishes_are_not() {
    crate::doctypes::register_doctypes();
    let state = test_state().await;
    // The production server installs the registry policies at boot; the
    // `All`-users read grant only exists once that has run.
    crate::authz::install_policies_with_role_permissions(&state.api.pool, None)
        .await
        .expect("install the registry policies");

    // A manager seeds one generation, owned by `owner-1`.
    ferris_document::with_user(Some(engine_user("owner-1", &["System Manager"])), async {
        let (status, _, _) = request_raw(
            app(&state),
            Method::PUT,
            "/api/desired-state/edge-3",
            Some(serde_json::json!({ "payload": { "ok": true } }).to_string()),
            &[],
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    })
    .await;

    // A plain user can read it...
    ferris_document::with_user(Some(engine_user("sales-1", &["Sales"])), async {
        let (status, _, body) = request_raw(
            app(&state),
            Method::GET,
            "/api/desired-state/edge-3",
            None,
            &[],
            &state,
            &["Sales"],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // ...but cannot publish.
        let (status, _, body) = request_raw(
            app(&state),
            Method::PUT,
            "/api/desired-state/edge-3",
            Some(serde_json::json!({ "payload": { "ok": false } }).to_string()),
            &[],
            &state,
            &["Sales"],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    })
    .await;
}
