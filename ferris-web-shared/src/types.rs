//! Re-exports of the canonical wire contract.
//!
//! Every payload the Desk sends or receives is defined in
//! `ferris_schema::contract`, not here. The definitions live in one crate that
//! both the server (`ferris-api`) and the browser compile against;
//! `ferris-web` re-exports this module as `api::types` so callers keep their
//! historical import paths.
//!
//! Nothing may be *defined* in this file: a new DTO added here would be a
//! contract with no server-side counterpart.

pub use ferris_schema::contract::list::{
    ApiResponse, FILTER_OPERATORS, Filter, FilterOp, ListRequest, Neighbors, filters_json,
};

/// The Connections panel payload, shared with `ferris-api`'s handler.
pub use ferris_schema::contract::meta::{ConnectionDocument, ConnectionGroup, Connections};

/// The Role Permissions Manager matrix, shared with `ferris-api`.
pub use ferris_schema::contract::meta::{CustomPermissionGrant, PermissionMatrix};

/// Per-document engagement state, shared with `ferris-document`'s stores and
/// the `POST /api/engagement/...` handlers.
pub use ferris_schema::contract::engagement::{DocComment, EngagementPage, LikeState};

/// A document's activity feed (`GET /api/activity/{doctype}/{name}`), shared
/// with `ferris-document`'s `ActivityStore`.
pub use ferris_schema::contract::activity::{ActivityEntry, ActivityKind};

/// Per-document collaboration side tables (assignments, attachments, shares),
/// shared with `ferris-document`'s stores and the `ferris-api` handlers.
pub use ferris_schema::contract::collaboration::{Assignment, Attachment, Share};

/// Runtime customization wire payloads (`GET/POST/DELETE /api/customization/...`),
/// shared with the `ferris-api` handlers and `ferris-schema`'s
/// `CustomizationStore`.
pub use ferris_schema::contract::customization::{
    CustomFieldSpec, Customization, ExportScope, PropertySetterSpec,
};

/// Desk-facing name for the list request body.
///
/// The Desk builds and serializes one request object; the REST handler
/// deserializes the very same struct. The alias exists because `ListRequest`
/// describes the request from the server's point of view while callers in the
/// views layer talk about list *params*.
pub use ferris_schema::contract::list::ListRequest as ListParams;
