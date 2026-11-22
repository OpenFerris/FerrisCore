//! The request/response envelope of `POST /api/graphql`.
//!
//! The shape is GraphQL's over HTTP, narrowed to what this endpoint answers:
//! `data` is omitted when the whole query failed to parse, `errors` is omitted
//! when nothing went wrong, and a per-field failure keeps both (the field is
//! `null` and its path is named).

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Incoming GraphQL request.
#[derive(Debug, Clone, Deserialize)]
pub struct GraphQlRequest {
    pub query: String,
    #[serde(default)]
    pub variables: Option<JsonValue>,
    #[serde(default)]
    pub operation_name: Option<String>,
}

/// A GraphQL error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQlError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<String>>,
}

/// GraphQL response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQlResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonValue>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<GraphQlError>,
}

impl GraphQlResponse {
    pub(super) fn error(message: impl Into<String>) -> Self {
        Self {
            data: None,
            errors: vec![GraphQlError {
                message: message.into(),
                path: None,
            }],
        }
    }
}
