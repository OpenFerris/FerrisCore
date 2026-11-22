//! The axum handler behind `POST /api/graphql`.
//!
//! It only unwraps the envelope: the variables must be a JSON object (anything
//! else is treated as "no variables"), and the answer is whatever
//! [`execute_query`] returns, always with HTTP 200 — GraphQL reports failure in
//! the body, not in the status line.

use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use serde_json::Value as JsonValue;
use std::sync::Arc;

use crate::router::ApiState;

use super::execution::execute_query;
use super::wire::GraphQlRequest;

/// axum handler for `POST /api/graphql`.
pub(crate) async fn graphql_handler(
    State(state): State<Arc<ApiState>>,
    Json(request): Json<GraphQlRequest>,
) -> Response {
    let variables = request
        .variables
        .and_then(|v| match v {
            JsonValue::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    Json(execute_query(&state.pool, &request.query, &variables).await).into_response()
}
