//! `POST /api/doctype` and `PUT /api/doctype/{name}`: the Desk builder's server
//! half. The policy and the DDL live in [`crate::doctype_admin`]; these are the
//! thin HTTP wrappers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{ApiState, error_response};
use ferris_schema::DoctypeMeta;

/// `POST /api/doctype` — create a runtime `DocType`.
pub(super) async fn create(
    State(state): State<Arc<ApiState>>,
    Json(meta): Json<DoctypeMeta>,
) -> Response {
    let Some(user) = ferris_document::current_user() else {
        return (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    };
    match crate::doctype_admin::create(&state.pool, &user, meta).await {
        Ok(meta) => (StatusCode::CREATED, Json(meta)).into_response(),
        Err(error) => error_response(error),
    }
}

/// `PUT /api/doctype/{name}` — edit a **custom** `DocType`.
pub(super) async fn update(
    State(state): State<Arc<ApiState>>,
    Path(name): Path<String>,
    Json(meta): Json<DoctypeMeta>,
) -> Response {
    let Some(user) = ferris_document::current_user() else {
        return (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    };
    match crate::doctype_admin::update(&state.pool, &user, &name, meta).await {
        Ok(meta) => Json(meta).into_response(),
        Err(error) => error_response(error),
    }
}
