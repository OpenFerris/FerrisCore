use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ferris_core::User;
use ferris_core::permissions::{Permission, Resource, SYSTEM_MANAGER_ROLE, check_permission};
use ferris_core::{Error, Value};
use ferris_schema::DoctypeMeta;

use super::{ApiState, error_response};

/// Whether `user` may read the metadata of `meta`.
///
/// Metadata is not the document: a caller can read a DocType's field list and
/// still hold no `Read` on any row. It is nevertheless schema disclosure — field
/// names, links, permissions and actions — so it follows the same `Read` rule
/// the engine applies to documents (API-09). `custom`/`hidden` DocTypes are
/// site configuration the Desk exposes to operators, not to every authenticated
/// user, so they are reserved to `System Manager` even when a policy would
/// grant `Read`.
fn may_read_metadata(user: &User, meta: &DoctypeMeta) -> bool {
    if (meta.custom || meta.hidden) && !user.has_role(SYSTEM_MANAGER_ROLE) {
        return false;
    }
    check_permission(
        user,
        &Resource::new("doctype", meta.name.clone()),
        Permission::Read,
    )
}

/// `GET /api/doctype/meta/{doctype}` — the effective metadata of a `DocType`.
///
/// An unknown `DocType` is a `404`, not `200 null`: the client must
/// be able to tell "no such `DocType`" from a malformed or empty body.
///
/// The metadata is loaded once and reused for the payload, so the permission
/// decision and what is returned cannot disagree.
pub(super) async fn meta(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
) -> Response {
    let Some(user) = ferris_document::current_user() else {
        return (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    };
    let meta = match crate::store::effective_meta(&state.pool, &doctype).await {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            return error_response(Error::Schema(ferris_core::SchemaError::DocTypeNotFound(
                doctype,
            )));
        }
        Err(e) => return error_response(e),
    };
    if !may_read_metadata(&user, &meta) {
        return error_response(Error::Authz(ferris_core::AuthzError::PermissionDenied {
            action: "read metadata".to_string(),
            resource: format!("doctype `{}`", meta.name),
        }));
    }
    match ferris_schema::registry::dyn_meta(&state.pool, &meta).await {
        Ok(payload) => Json(payload).into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /api/doctype/actions/{doctype}` — the DocType Action rows.
pub(super) async fn doc_actions(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
) -> Response {
    match crate::doc_meta::doc_actions(&state.pool, &doctype).await {
        Ok(actions) => Json(serde_json::json!({ "actions": actions })).into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /api/doctype/connections/{doctype}/{name}` — the Connections panel.
pub(super) async fn connections(
    State(state): State<Arc<ApiState>>,
    Path((doctype, name)): Path<(String, String)>,
) -> Response {
    match crate::doc_meta::connections(&state.pool, &doctype, &name).await {
        Ok(connections) => Json(connections).into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /api/doctype/permissions/{doctype}` — the caller's custom permissions.
pub(super) async fn permission_types(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
) -> Response {
    let Some(user) = ferris_document::current_user() else {
        return (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    };
    match crate::doc_meta::permission_inventory(&state.pool, &user, &doctype).await {
        Ok(custom) => Json(serde_json::json!({ "custom": custom })).into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /api/doctype/permissions/{doctype}/manage` — the role × type matrix.
pub(super) async fn permission_matrix(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
) -> Response {
    let Some(user) = ferris_document::current_user() else {
        return (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    };
    match crate::permission_admin::matrix(&state.pool, &user, &doctype).await {
        Ok(matrix) => Json(matrix).into_response(),
        Err(e) => error_response(e),
    }
}

/// `PUT /api/doctype/permissions/{doctype}/manage` — grant or revoke one type.
pub(super) async fn set_custom_permission(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(user) = ferris_document::current_user() else {
        return (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    };
    let role = body.get("role").and_then(Value::as_str).unwrap_or_default();
    let perm_type = body
        .get("perm_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let granted = body
        .get("granted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Err(e) =
        crate::permission_admin::set_grant(&state.pool, &user, &doctype, role, perm_type, granted)
            .await
    {
        return error_response(e);
    }
    // A permission change is only real once the installed policy reflects it;
    // rebuild through the same path a boot uses, so the matrix and the engine
    // cannot disagree until the next restart.
    match crate::authz::install_policies_with_role_permissions(
        &state.pool,
        state.policy_path.as_deref(),
    )
    .await
    {
        Ok(_) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => error_response(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(roles: &[&str]) -> User {
        let mut user = User::system_user("reader@local".to_string(), "Reader".to_string());
        user.roles = roles.iter().map(|role| (*role).to_string()).collect();
        user
    }

    /// API-09: `custom`/`hidden` DocTypes are operator configuration, so a
    /// plain authenticated user cannot read their metadata even if a policy
    /// would grant `Read` on the doctype.
    #[test]
    fn hidden_and_custom_metadata_is_reserved_to_system_manager() {
        let mut meta = DoctypeMeta {
            name: "Metadata Probe".to_string(),
            ..Default::default()
        };
        meta.hidden = true;
        assert!(!may_read_metadata(&user(&[]), &meta));
        assert!(!may_read_metadata(&user(&["Metadata Probe Role"]), &meta));
        assert!(may_read_metadata(&user(&[SYSTEM_MANAGER_ROLE]), &meta));

        meta.hidden = false;
        meta.custom = true;
        assert!(!may_read_metadata(&user(&["Metadata Probe Role"]), &meta));
        assert!(may_read_metadata(&user(&[SYSTEM_MANAGER_ROLE]), &meta));
    }

    /// A regular DocType needs `Read` like its documents do; the default
    /// engine denies a role that no policy grants.
    #[test]
    fn regular_metadata_requires_read() {
        let meta = DoctypeMeta {
            name: "Metadata Probe".to_string(),
            ..Default::default()
        };
        assert!(!may_read_metadata(&user(&[]), &meta));
        assert!(!may_read_metadata(&user(&["Metadata Probe Role"]), &meta));
        assert!(may_read_metadata(&user(&[SYSTEM_MANAGER_ROLE]), &meta));
    }
}
