//! Shared API types.
//!
//! The Desk-facing list payload lives in `ferris-schema::contract` and is
//! re-exported from here, so the REST handlers and the Desk client name the
//! same type instead of keeping a hand-synced copy. This module keeps only the
//! server-side shapes that have no client counterpart.

use ferris_core::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use ferris_schema::contract::list::{ApiResponse, ListRequest};

/// Legacy `GET /api/resource/{doctype}` query parameters (pagination + simple
/// equality filters).
///
/// The Desk does not use this path — it posts a [`ListRequest`] instead — but
/// it is still part of the public REST surface for external callers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListParams {
    /// Equality filters applied to the parsed document.
    #[serde(default)]
    pub filters: Option<HashMap<String, Value>>,
    /// Optional field projection.
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
    /// Ordering: any document field, optionally followed by `asc`/`desc`
    /// (`order_by=qty desc`). The engine pushes column-backed fields
    /// (`name`, `owner`, `docstatus`, `creation`, `modified`) into SQL and
    /// falls back to an in-Rust stable sort for blob fields, so this is not
    /// limited to a fixed allow-list. The historical aliases `created_at` and
    /// `updated_at` are accepted and mapped to `creation`/`modified`; when no
    /// direction is written on `order_by`, [`ListParams::order`] is applied.
    #[serde(default)]
    pub order_by: Option<String>,
    /// Direction applied to `order_by` when the expression carries none.
    #[serde(default)]
    pub order: Option<Order>,
    /// Case-insensitive substring match over the document's string values.
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    Asc,
    Desc,
}

/// Generic list envelope for the legacy query path.
///
/// Besides pagination (`limit`/`offset`) it carries the 1-based `page` and the
/// page size, so this path answers with the same envelope
/// ([`ApiResponse`]) the Desk path uses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
    pub page: u64,
    pub page_size: u32,
}

/// JSON error body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    pub message: String,
}

impl ApiError {
    pub fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
        }
    }
}
