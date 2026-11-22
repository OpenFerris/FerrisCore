use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ferris_auth::docperm::{DocPermRow, DocPermStore};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::Error;

use super::{ApiState, error_response};

// --- Role Permissions Builder ------------------------------------------------
//
// Frappe's builder writes `DocPerm` rows from the desk; these routes are the
// same surface over the persisted store (`ferris_auth::docperm`). Every mutating
// route reinstalls the engine afterwards through the same rebuild the boot path
// uses, so an edit is live immediately and can never drift from what the next
// boot would start with.

/// Admin gate: the desk's permission surface only answers to the roles that can
/// already restructure the engine. Frappe gates Role Permissions Builder behind
/// Administrator / System Manager the same way.
pub(super) fn is_permissions_admin(user: &AuthenticatedUser) -> bool {
    user.0.has_role("System Manager") || user.0.has_role("Administrator")
}

fn permissions_forbidden() -> Response {
    error_response(Error::Authz(ferris_core::AuthzError::PermissionDenied {
        action: "manage role permissions".to_string(),
        resource: "tabDocPerm".to_string(),
    }))
}

#[derive(serde::Deserialize)]
pub(super) struct RolePermQuery {
    doctype: Option<String>,
}

pub(super) async fn list_role_permissions(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Query(query): Query<RolePermQuery>,
) -> Response {
    if !is_permissions_admin(&user) {
        return permissions_forbidden();
    }
    let store = DocPermStore::new(state.pool.clone());
    if let Err(e) = store.init().await {
        return error_response(e);
    }
    let rows = match &query.doctype {
        Some(doctype) => store.list(doctype).await,
        None => store.list_all().await,
    };
    match rows {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => error_response(e),
    }
}

/// The response body of an upsert: a `DocPermRow` document, or an array.
pub(super) async fn upsert_role_permission(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Json(row): Json<DocPermRow>,
) -> Response {
    if !is_permissions_admin(&user) {
        return permissions_forbidden();
    }
    if row.doctype.trim().is_empty() || row.role.trim().is_empty() {
        return error_response(Error::Validation(ferris_core::ValidationError::Invalid(
            "Role Permissions rows need both `doctype` and `role`".to_string(),
        )));
    }
    let store = DocPermStore::new(state.pool.clone());
    if let Err(e) = store.init().await {
        return error_response(e);
    }
    if let Err(e) = store.upsert(&row).await {
        return error_response(e);
    }
    if let Err(e) = crate::authz::install_policies_with_role_permissions(
        &state.pool,
        state.policy_path.as_deref(),
    )
    .await
    {
        return error_response(e);
    }
    // Echo the live rows for this DocType, so the caller sees what the engine
    // now runs with rather than trusting the echo of what they sent.
    match store.list(&row.doctype).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn delete_role_permission(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((doctype, role)): Path<(String, String)>,
) -> Response {
    if !is_permissions_admin(&user) {
        return permissions_forbidden();
    }
    let store = DocPermStore::new(state.pool.clone());
    if let Err(e) = store.init().await {
        return error_response(e);
    }
    if let Err(e) = store.delete(&doctype, &role).await {
        return error_response(e);
    }
    match crate::authz::install_policies_with_role_permissions(
        &state.pool,
        state.policy_path.as_deref(),
    )
    .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(e),
    }
}
