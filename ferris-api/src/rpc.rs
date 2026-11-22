//! JSON-RPC 2.0 and batch dispatch over the [`MethodRegistry`].
//!
//! Mirrors Frappe's `frappe.call()` and `batch` endpoint.
//!
//! JSON-RPC keeps its `200` transport status even for a failed call — the
//! protocol carries the failure in the `error` object — so [`dispatch`] cannot
//! reuse the REST layer's status-based sanitising. [`safe_error_message`] is the
//! one sanitiser these envelopes (single, batch and GraphQL) share: an internal
//! failure is logged and answered with a fixed message instead of leaking SQL,
//! table names or wrapped causes.

use ferris_core::{Error, Value};
use serde::{Deserialize, Serialize};

use crate::method::MethodRegistry;

/// The largest batch `/api/batch` accepts.
///
/// One request used to fan out into an unbounded number of engine calls, so a
/// single cheap HTTP request could drive arbitrary work. [`dispatch_batch`]
/// itself has no limit (it is a plain loop); the route refuses a larger batch
/// before dispatching any of it.
pub const MAX_BATCH_SIZE: usize = 20;

/// A caller-facing message for `error`.
///
/// `Validation`, `Authz`, `NotFound` and `Conflict` describe the caller's own
/// input or the state of the resource, so their text is returned. Everything
/// else (database/driver errors, SQL, storage, configuration, wrapped causes) is
/// ours: it is logged here, once, and the caller sees `"internal error"`.
#[must_use]
pub fn safe_error_message(error: &Error) -> String {
    use ferris_core::error::TenantError;
    use ferris_core::{
        AuthError, DocumentError, EmailError, FileError, JobError, SchemaError, WorkflowError,
    };

    let safe = matches!(
        error,
        Error::Validation(_)
            | Error::Authz(_)
            | Error::Document(DocumentError::NotFound { .. } | DocumentError::AlreadyExists { .. })
            | Error::Schema(SchemaError::DocTypeNotFound(_) | SchemaError::DuplicateField { .. })
            | Error::Auth(AuthError::NotFound(_))
            | Error::File(FileError::NotFound(_))
            | Error::Job(JobError::NotFound(_) | JobError::QueueNotFound(_))
            | Error::Email(EmailError::NotFound(_))
            | Error::Workflow(WorkflowError::NotFound(_) | WorkflowError::AlreadyExists(_))
            | Error::Tenant(TenantError::NotFound(_) | TenantError::AlreadyExists(_))
    );
    if safe {
        error.to_string()
    } else {
        tracing::error!(error = %error, "suppressed internal error detail in an RPC response");
        "internal error".to_string()
    }
}

/// A JSON-RPC 2.0 request (also used as one entry of a batch).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jsonrpc: Option<String>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: Some("2.0".to_string()),
            method: method.into(),
            params: Some(params),
            id: None,
        }
    }
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    fn failure(id: Option<Value>, code: i64, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError { code, message }),
            id,
        }
    }
}

/// Execute a single JSON-RPC request.
pub async fn dispatch(registry: &MethodRegistry, request: JsonRpcRequest) -> JsonRpcResponse {
    let id = request.id.clone();
    match registry
        .call(&request.method, request.params.unwrap_or(Value::Null))
        .await
    {
        Ok(result) => JsonRpcResponse::success(id, result),
        Err(e) => {
            let code = match e {
                Error::Document(ferris_core::DocumentError::NotFound { .. }) => -32601, // method not found
                Error::Validation(_) => -32602, // invalid params
                _ => -32603,                    // internal error
            };
            JsonRpcResponse::failure(id, code, safe_error_message(&e))
        }
    }
}

/// The refusal a batch over [`MAX_BATCH_SIZE`] gets, without dispatching any
/// entry.
///
/// `-32600` is JSON-RPC's "invalid request": the batch envelope itself is
/// invalid, not one particular call.
#[must_use]
pub fn batch_too_large(size: usize) -> JsonRpcResponse {
    JsonRpcResponse::failure(
        None,
        -32600,
        format!("invalid request: batch of {size} exceeds the limit of {MAX_BATCH_SIZE}"),
    )
}

/// Execute a batch of JSON-RPC requests. Every entry always produces a response.
///
/// The caller enforces [`MAX_BATCH_SIZE`]; keeping the loop unbounded here means
/// [`dispatch_batch`] has no hidden partial-success semantics.
pub async fn dispatch_batch(
    registry: &MethodRegistry,
    requests: Vec<JsonRpcRequest>,
) -> Vec<JsonRpcResponse> {
    let mut responses = Vec::with_capacity(requests.len());
    for request in requests {
        responses.push(dispatch(registry, request).await);
    }
    responses
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;

    struct Echo;

    #[async_trait]
    impl crate::method::MethodHandler for Echo {
        async fn call(&self, params: Value) -> ferris_core::Result<Value> {
            Ok(params)
        }
    }

    fn registry() -> MethodRegistry {
        let registry = MethodRegistry::new();
        // These tests exercise the dispatch envelope, not authorization, so the
        // echo method is explicitly public.
        registry
            .register_base_with_policy(
                "example.echo",
                Arc::new(Echo),
                crate::method::MethodPolicy::guest(),
            )
            .unwrap();
        registry
    }

    #[tokio::test]
    async fn test_dispatch_success() {
        let response = dispatch(
            &registry(),
            JsonRpcRequest::new("example.echo", Value::from("hi")),
        )
        .await;
        assert_eq!(response.result, Some(Value::from("hi")));
        assert!(response.error.is_none());
        assert_eq!(response.jsonrpc, "2.0");
    }

    #[tokio::test]
    async fn test_dispatch_method_not_found() {
        let response = dispatch(
            &registry(),
            JsonRpcRequest::new("missing.method", Value::Null),
        )
        .await;
        assert_eq!(response.error.as_ref().unwrap().code, -32601);
    }

    #[tokio::test]
    async fn test_dispatch_batch() {
        let requests = vec![
            JsonRpcRequest::new("example.echo", Value::from(1)),
            JsonRpcRequest::new("missing.method", Value::Null),
            JsonRpcRequest::new("example.echo", Value::from("three")),
        ];
        let responses = dispatch_batch(&registry(), requests).await;
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0].result, Some(Value::from(1)));
        assert!(responses[1].error.is_some());
        assert_eq!(responses[2].result, Some(Value::from("three")));
    }

    /// API-02/SEC-12: a driver error must not put its SQL/path text in the
    /// JSON-RPC envelope, while a validation error still describes the input.
    #[tokio::test]
    async fn internal_errors_do_not_leak_their_text() {
        let internal = Error::Database(ferris_core::DatabaseError::Query(
            "SELECT secret FROM tab_api_key WHERE 1=1".to_string(),
        ));
        assert_eq!(safe_error_message(&internal), "internal error");

        let validation = Error::Validation(ferris_core::ValidationError::Required("name".into()));
        assert!(safe_error_message(&validation).contains("name"));

        let not_found = Error::Document(ferris_core::DocumentError::NotFound {
            doctype: "ToDo".into(),
            name: "nope".into(),
        });
        assert!(safe_error_message(&not_found).contains("ToDo"));
    }

    /// API-14: the batch cap is a hard, explicit envelope refusal.
    #[test]
    fn a_batch_over_the_limit_is_refused_as_invalid_request() {
        assert_eq!(MAX_BATCH_SIZE, 20);
        let refusal = batch_too_large(MAX_BATCH_SIZE + 1);
        let error = refusal.error.expect("a refusal carries an error");
        assert_eq!(error.code, -32600);
        assert!(refusal.result.is_none());
        assert!(error.message.contains("batch"));
    }
}
