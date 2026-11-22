//! The native module contract: mounting, authentication and OpenAPI.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::routing::get;
use tower::ServiceExt;

use super::harness::{app_with_modules, request, test_state, test_state_with_openapi};
use crate::module::{Module, ModuleState, OpenApiFragment};

struct PingModule;

impl Module for PingModule {
    fn name(&self) -> &'static str {
        "ping"
    }

    fn routes(&self) -> Router<ModuleState> {
        Router::new().route("/ping", get(|| async { "pong" }))
    }

    fn openapi(&self) -> OpenApiFragment {
        OpenApiFragment {
            paths: vec![(
                "/api/ping".to_string(),
                serde_json::json!({
                    "get": {
                        "summary": "Ping",
                        "responses": { "200": { "description": "pong" } }
                    }
                }),
            )],
        }
    }
}

#[tokio::test]
async fn a_module_is_mounted_under_api_behind_the_auth_layer() {
    let state = test_state().await;
    let modules: Vec<Arc<dyn Module>> = vec![Arc::new(PingModule)];

    let response = app_with_modules(&state, &modules)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/ping")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a module route must inherit the host's auth layer"
    );

    let (status, body) = request(
        app_with_modules(&state, &modules),
        Method::GET,
        "/api/ping",
        None,
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "pong");
}

#[tokio::test]
async fn a_module_is_absent_when_it_is_not_registered() {
    let state = test_state().await;
    let (status, _) = request(
        app_with_modules(&state, &[]),
        Method::GET,
        "/api/ping",
        None,
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_module_contributes_to_the_openapi_document() {
    let state = test_state_with_openapi(
        ferris_core::FeatureFlags::default(),
        vec![PingModule.openapi()],
    )
    .await;
    let (status, body) = request(
        app_with_modules(&state, &[]),
        Method::GET,
        "/api/openapi.json",
        None,
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let spec: serde_json::Value = serde_json::from_str(&body).expect("OpenAPI JSON");
    assert!(
        spec["paths"]["/api/ping"]["get"].is_object(),
        "the module path must reach the spec: {body}"
    );
}
