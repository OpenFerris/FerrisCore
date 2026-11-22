use axum::http::{Method, StatusCode};
use ferris_core::FeatureFlags;

use super::harness::{app, request, test_state, test_state_with};

#[tokio::test]
async fn a_disabled_search_feature_removes_the_route() {
    let enabled = test_state().await;
    let (status, body) = request(
        app(&enabled),
        Method::GET,
        "/api/search?q=ferris",
        None,
        &enabled,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut features = FeatureFlags::default();
    features.search = false;
    let disabled = test_state_with(features).await;
    let (status, _) = request(
        app(&disabled),
        Method::GET,
        "/api/search?q=ferris",
        None,
        &disabled,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
