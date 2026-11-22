use std::sync::Arc;

use axum::{
    Json,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::Value;

use crate::store::DocStore;
use crate::types::{ListParams, ListRequest};

use super::{ApiState, error_response};

pub(super) async fn list(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
    Query(params): Query<ListParams>,
) -> Response {
    match DocStore::list(&state.pool, &doctype, &params).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => error_response(e),
    }
}

/// The one `/api/resource/{doctype}/list` handler. It always runs on the
/// authenticated router, so the caller is known and [`ListRequest::only_liked`]
/// can be resolved against `tab_like`.
pub(super) async fn list_requested_authed<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path(doctype): Path<String>,
    Json(req): Json<ListRequest>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    respond_list(&state, &doctype, &req, Some(&user.0.id)).await
}

async fn respond_list(
    state: &Arc<ApiState>,
    doctype: &str,
    req: &ListRequest,
    user: Option<&str>,
) -> Response {
    match DocStore::list_requested(&state.pool, doctype, req, user).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn get_one(
    State(state): State<Arc<ApiState>>,
    Path((doctype, name)): Path<(String, String)>,
) -> Response {
    match DocStore::get(&state.pool, &doctype, &name).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /api/resource/{doctype}/{name}/neighbors` — the previous/next docname in
/// list order, for the form's navigation controls.
pub(super) async fn neighbors(
    State(state): State<Arc<ApiState>>,
    Path((doctype, name)): Path<(String, String)>,
) -> Response {
    match DocStore::neighbors(&state.pool, &doctype, &name).await {
        Ok(neighbors) => Json(neighbors).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn create(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    match DocStore::create(&state.pool, &doctype, body).await {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn update(
    State(state): State<Arc<ApiState>>,
    Path((doctype, name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    match DocStore::update(&state.pool, &doctype, &name, body).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn remove(
    State(state): State<Arc<ApiState>>,
    Path((doctype, name)): Path<(String, String)>,
) -> Response {
    match DocStore::delete(&state.pool, &doctype, &name).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn submit(
    State(state): State<Arc<ApiState>>,
    Path((doctype, name)): Path<(String, String)>,
) -> Response {
    match DocStore::set_docstatus(
        &state.pool,
        &doctype,
        &name,
        ferris_document::DocStatus::Submitted,
    )
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn cancel(
    State(state): State<Arc<ApiState>>,
    Path((doctype, name)): Path<(String, String)>,
) -> Response {
    match DocStore::set_docstatus(
        &state.pool,
        &doctype,
        &name,
        ferris_document::DocStatus::Cancelled,
    )
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn amend(
    State(state): State<Arc<ApiState>>,
    Path((doctype, name)): Path<(String, String)>,
) -> Response {
    match DocStore::amend(&state.pool, &doctype, &name).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(e),
    }
}

/// `POST /api/resource/{doctype}/restore/{token}` — restore a soft-deleted
/// document from `Deleted Document` while its original name is free.
pub(super) async fn restore(
    State(state): State<Arc<ApiState>>,
    Path((doctype, token)): Path<(String, String)>,
) -> Response {
    match DocStore::restore(&state.pool, &doctype, &token).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /api/single/{doctype}` — read the one document of a Single `DocType`,
/// creating it with its field defaults on first access.
pub(super) async fn get_single(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
) -> Response {
    match DocStore::get_single(&state.pool, &doctype).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(e),
    }
}

/// `PUT /api/single/{doctype}` — save the one document of a Single `DocType`.
pub(super) async fn save_single(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    match DocStore::save_single(&state.pool, &doctype, body).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(e),
    }
}

/// `POST /api/resource/{doctype}/{name}/rename` — move a document's primary key.
pub(super) async fn rename(
    State(state): State<Arc<ApiState>>,
    Path((doctype, name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let new_name = body
        .get("new_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match DocStore::rename(&state.pool, &doctype, &name, new_name).await {
        Ok(document) => Json(document).into_response(),
        Err(e) => error_response(e),
    }
}
