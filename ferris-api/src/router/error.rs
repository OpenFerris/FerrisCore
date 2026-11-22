use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ferris_core::Error;

use crate::types::ApiError;

pub fn error_response(e: Error) -> Response {
    let status = match e {
        Error::Document(ferris_core::DocumentError::NotFound { .. })
        | Error::Schema(ferris_core::SchemaError::DocTypeNotFound(_))
        | Error::Auth(ferris_core::AuthError::NotFound(_))
        | Error::File(ferris_core::FileError::NotFound(_))
        | Error::Job(
            ferris_core::JobError::NotFound(_) | ferris_core::JobError::QueueNotFound(_),
        )
        | Error::Search(ferris_core::SearchError::IndexNotFound(_))
        | Error::Workflow(ferris_core::WorkflowError::NotFound(_))
        | Error::Email(ferris_core::EmailError::NotFound(_))
        | Error::Tenant(ferris_core::error::TenantError::NotFound(_)) => StatusCode::NOT_FOUND,
        Error::Validation(_) | Error::Authz(ferris_core::AuthzError::InvalidPolicy(_)) => {
            StatusCode::BAD_REQUEST
        }
        Error::Document(ferris_core::DocumentError::AlreadyExists { .. })
        | Error::Workflow(ferris_core::WorkflowError::AlreadyExists(_))
        | Error::Tenant(ferris_core::error::TenantError::AlreadyExists(_))
        | Error::Schema(ferris_core::SchemaError::DuplicateField { .. }) => StatusCode::CONFLICT,
        Error::Auth(ferris_core::AuthError::TooManySessions(_)) => StatusCode::TOO_MANY_REQUESTS,
        Error::Auth(_) => StatusCode::UNAUTHORIZED,
        Error::Authz(_) => StatusCode::FORBIDDEN,
        // A secret that cannot be recovered is a configuration conflict the
        // operator has to resolve (re-enter the value, install the retired key),
        // not an internal fault — but it is also never silently retried.
        Error::Secret(
            ferris_core::SecretError::NotRecoverable { .. }
            | ferris_core::SecretError::UnknownKeyId { .. },
        ) => StatusCode::CONFLICT,
        // A Single DocType has no separate documents: `POST`/`DELETE` on its
        // resource route is a method that does not apply, not a server fault.
        Error::Document(
            ferris_core::DocumentError::SingleNotCreatable { .. }
            | ferris_core::DocumentError::SingleNotDeletable { .. },
        ) => StatusCode::METHOD_NOT_ALLOWED,
        // A call that ran out of time (a plugin method that hit its WASM
        // deadline, a job timeout surfaced through an RPC) is a gateway
        // timeout, not a defect in the request: the caller may retry it.
        Error::Job(ferris_core::JobError::Timeout) => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let label = status
        .canonical_reason()
        .unwrap_or("error")
        .to_lowercase()
        .replace(' ', "_");
    // A 5xx means *we* broke: the error's text carries SQL, paths, table names,
    // pool state or a stack of wrapped causes, and none of that is the caller's
    // business. It goes to the log, and the response says only that something
    // failed. 4xx keeps its message: it describes the caller's own input, and a
    // client needs it to fix the request.
    let (label, message) = if status.is_server_error() {
        tracing::error!(
            status = status.as_u16(),
            error = %e,
            "request failed with a server error"
        );
        ("internal_error".to_string(), "internal error".to_string())
    } else {
        (label, e.to_string())
    };
    let mut response = (status, Json(ApiError::new(label, message))).into_response();

    // Add WWW-Authenticate header for 401 responses. Both values are static,
    // so `from_static` builds them without a fallible parse.
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            axum::http::header::WWW_AUTHENTICATE,
            axum::http::HeaderValue::from_static("Bearer"),
        );
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
    }

    response
}
