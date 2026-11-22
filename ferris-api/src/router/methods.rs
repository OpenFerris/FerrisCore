use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ferris_core::Value;

use crate::rpc::{JsonRpcRequest, MAX_BATCH_SIZE, batch_too_large, dispatch, dispatch_batch};

use super::{ApiState, error_response};

pub(super) async fn call_method(
    State(state): State<Arc<ApiState>>,
    Path(method): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    match state.methods.call(&method, body).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn jsonrpc(
    State(state): State<Arc<ApiState>>,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    Json(dispatch(&state.methods, request).await).into_response()
}

/// `POST /api/batch` — up to [`MAX_BATCH_SIZE`] JSON-RPC calls in one request.
///
/// An oversized batch is refused **before** any entry runs: a `400` whose body
/// is a JSON-RPC `-32600` error. Return a partial answer instead and the caller
/// cannot tell which entries ran, and a failed envelope must not have side
/// effects.
pub(super) async fn batch(
    State(state): State<Arc<ApiState>>,
    Json(requests): Json<Vec<JsonRpcRequest>>,
) -> Response {
    if requests.len() > MAX_BATCH_SIZE {
        tracing::warn!(
            size = requests.len(),
            limit = MAX_BATCH_SIZE,
            "refused an oversized batch"
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(batch_too_large(requests.len())),
        )
            .into_response();
    }
    Json(dispatch_batch(&state.methods, requests).await).into_response()
}

pub(super) async fn openapi(State(state): State<Arc<ApiState>>) -> Response {
    let mut spec = crate::openapi::openapi_spec(
        "Ferris API",
        env!("CARGO_PKG_VERSION"),
        &state.methods.names(),
    );
    crate::openapi::merge_fragments(&mut spec, &state.openapi_fragments);
    Json(spec).into_response()
}
