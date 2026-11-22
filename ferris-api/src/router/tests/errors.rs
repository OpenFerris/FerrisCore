use axum::http::StatusCode;
use ferris_core::Error;

use super::super::error_response;

/// The one rule every 5xx follows:
/// the body says it failed, the log has the reason.
#[tokio::test]
async fn server_errors_do_not_leak_the_internal_reason() {
    let leaked = "select * from tab_api_key where secret = 'x'";
    let response = error_response(Error::Database(ferris_core::error::DatabaseError::Query(
        leaked.to_string(),
    )));
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "internal_error");
    assert_eq!(json["message"], "internal error");
    assert!(!text.contains("tab_api_key"), "{text}");
    assert!(!text.contains(leaked), "{text}");
}

/// A 4xx describes the caller's input; the client needs it to fix the
/// request, so it is not redacted.
#[tokio::test]
async fn client_errors_keep_their_message() {
    let response = error_response(Error::Validation(ferris_core::ValidationError::Invalid(
        "field `title` is required".to_string(),
    )));
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let message = json["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("field `title` is required"),
        "a 4xx must keep the caller-facing detail: {message}"
    );
}
