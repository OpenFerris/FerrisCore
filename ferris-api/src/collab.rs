//! Desk endpoints for a document's collaboration side tables: assignments,
//! attachments and shares.
//!
//! All three live under `/api/engagement/{doctype}/{name}/…` for the same
//! reason likes and comments do: they act on side tables keyed by
//! `(doctype, docname, …)`, not on the document itself, so there is no document
//! field to address.
//!
//! Reads require `Read` on the document; assignments and attachments require
//! `Write`; mutating the share list requires `Share`. The checks are
//! `crate::store::ensure_readable` / `ensure_writable` / `ensure_shareable`,
//! which resolve the policy through the engine rather than restating it, so a
//! new member of the side tables cannot ship with a weaker gate than the
//! document it hangs off.

use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_document::{AssignmentStore, AttachmentStore, DocShare, ShareManager, SharePermission};
use ferris_schema::contract::collaboration::Share;
use serde::Deserialize;

use crate::ApiState;

/// Reject a malformed doctype/name, then require `Read`. Returns `None` when the
/// caller may proceed, or the response to send back when they may not.
async fn guard_read(pool: &ferris_db::Db, doctype: &str, name: &str) -> Option<Response> {
    if let Err(e) = crate::store::validate_doctype(doctype) {
        return Some(crate::router::error_response(e));
    }
    if let Err(e) = crate::store::validate_name(name) {
        return Some(crate::router::error_response(e));
    }
    match crate::store::ensure_readable(pool, doctype, name).await {
        Ok(()) => None,
        Err(e) => Some(crate::router::error_response(e)),
    }
}

/// Reject a malformed doctype/name, then require `Write`.
async fn guard_write(pool: &ferris_db::Db, doctype: &str, name: &str) -> Option<Response> {
    if let Err(e) = crate::store::validate_doctype(doctype) {
        return Some(crate::router::error_response(e));
    }
    if let Err(e) = crate::store::validate_name(name) {
        return Some(crate::router::error_response(e));
    }
    match crate::store::ensure_writable(pool, doctype, name).await {
        Ok(()) => None,
        Err(e) => Some(crate::router::error_response(e)),
    }
}

/// Reject a malformed doctype/name, then require `Permission::Share`.
///
/// Frappe gates the share list with its own `share` right, not with `Write`: an
/// editor must not be able to hand the document to somebody else.
async fn guard_share(pool: &ferris_db::Db, doctype: &str, name: &str) -> Option<Response> {
    if let Err(e) = crate::store::validate_doctype(doctype) {
        return Some(crate::router::error_response(e));
    }
    if let Err(e) = crate::store::validate_name(name) {
        return Some(crate::router::error_response(e));
    }
    match crate::store::ensure_shareable(pool, doctype, name).await {
        Ok(()) => None,
        Err(e) => Some(crate::router::error_response(e)),
    }
}

// --- assignments -----------------------------------------------------------

/// `GET /api/engagement/{doctype}/{name}/assignments`
pub async fn list_assignments<S>(
    State(state): State<Arc<ApiState>>,
    _user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Some(response) = guard_read(&state.pool, &doctype, &name).await {
        return response;
    }
    match AssignmentStore::new(state.pool.clone())
        .list(&doctype, &name)
        .await
    {
        Ok(assignments) => Json(assignments).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `POST /api/engagement/{doctype}/{name}/assignments` — assign `user`.
pub async fn add_assignment<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
    Json(body): Json<AssignmentBody>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Some(response) = guard_write(&state.pool, &doctype, &name).await {
        return response;
    }
    match AssignmentStore::new(state.pool.clone())
        .assign(&doctype, &name, &body.user, &user.0.id)
        .await
    {
        Ok(assignment) => (StatusCode::CREATED, Json(assignment)).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `DELETE /api/engagement/{doctype}/{name}/assignments/{user}`
pub async fn remove_assignment<S>(
    State(state): State<Arc<ApiState>>,
    _user: AuthenticatedUser,
    Path((doctype, name, assigned_to)): Path<(String, String, String)>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Some(response) = guard_write(&state.pool, &doctype, &name).await {
        return response;
    }
    match AssignmentStore::new(state.pool.clone())
        .unassign(&doctype, &name, &assigned_to)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// Assignment request body. `user` is the assignee.
#[derive(Debug, Default, Deserialize)]
pub struct AssignmentBody {
    #[serde(default)]
    user: String,
}

// --- attachments -----------------------------------------------------------

/// `GET /api/engagement/{doctype}/{name}/attachments`
pub async fn list_attachments<S>(
    State(state): State<Arc<ApiState>>,
    _user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Some(response) = guard_read(&state.pool, &doctype, &name).await {
        return response;
    }
    match AttachmentStore::new(state.pool.clone())
        .list(&doctype, &name)
        .await
    {
        Ok(attachments) => Json(attachments).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `POST /api/engagement/{doctype}/{name}/attachments` — register a stored file
/// as an attachment.
pub async fn add_attachment<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
    Json(body): Json<AttachmentBody>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Some(response) = guard_write(&state.pool, &doctype, &name).await {
        return response;
    }
    match AttachmentStore::new(state.pool.clone())
        .add(
            &doctype,
            &name,
            &user.0.id,
            &body.file_name,
            &body.file_url,
            body.file_size,
        )
        .await
    {
        Ok(attachment) => (StatusCode::CREATED, Json(attachment)).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `DELETE /api/engagement/{doctype}/{name}/attachments/{id}`
pub async fn remove_attachment<S>(
    State(state): State<Arc<ApiState>>,
    _user: AuthenticatedUser,
    Path((doctype, name, id)): Path<(String, String, String)>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Some(response) = guard_write(&state.pool, &doctype, &name).await {
        return response;
    }
    match AttachmentStore::new(state.pool.clone())
        .remove(&doctype, &name, &id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// Attachment request body. `file_url` is required and points at where the
/// bytes live (an object-store URL or a `ferris-files` download path).
// Every field describes the same uploaded file, so they are all `file_*`: the
// `file_` prefix is what keeps `file_url` (where the bytes live) from being
// confused with the document's own `url` fields.
#[allow(
    clippy::struct_field_names,
    reason = "all three fields describe one file"
)]
#[derive(Debug, Default, Deserialize)]
pub struct AttachmentBody {
    #[serde(default)]
    file_name: String,
    #[serde(default)]
    file_url: String,
    #[serde(default)]
    file_size: u64,
}

// --- shares ----------------------------------------------------------------

/// Build the share manager for this request and make sure its table exists.
///
/// `ShareManager::with_db` is cheap (it only holds the pool) and `init` is
/// idempotent, the same shape the other side-table stores use, so there is no
/// shared manager to keep in sync with `ApiState`.
async fn share_manager(state: &ApiState) -> ferris_core::Result<ShareManager> {
    let manager = ShareManager::with_db(state.pool.clone());
    manager.init().await?;
    Ok(manager)
}

/// `GET /api/engagement/{doctype}/{name}/shares`
pub async fn list_shares<S>(
    State(state): State<Arc<ApiState>>,
    _user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Some(response) = guard_read(&state.pool, &doctype, &name).await {
        return response;
    }
    let manager = match share_manager(&state).await {
        Ok(manager) => manager,
        Err(e) => return crate::router::error_response(e),
    };
    match manager.get_shares(&doctype, &name).await {
        Ok(shares) => Json(shares.into_iter().map(to_wire).collect::<Vec<_>>()).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `POST /api/engagement/{doctype}/{name}/shares` — grant `user` access.
///
/// A grant with no permission flag defaults to `Read`: sharing a document while
/// granting nothing would create a row that changes no outcome, so the caller's
/// silence is read as "let them see it".
pub async fn add_share<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
    Json(body): Json<ShareBody>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Some(response) = guard_share(&state.pool, &doctype, &name).await {
        return response;
    }
    let mut permissions = Vec::new();
    if body.read {
        permissions.push(SharePermission::Read);
    }
    if body.write {
        permissions.push(SharePermission::Write);
    }
    if body.share {
        permissions.push(SharePermission::Share);
    }
    if permissions.is_empty() {
        permissions.push(SharePermission::Read);
    }

    let manager = match share_manager(&state).await {
        Ok(manager) => manager,
        Err(e) => return crate::router::error_response(e),
    };
    match manager
        .share(&doctype, &name, &body.user, permissions, &user.0.id)
        .await
    {
        Ok(share) => (StatusCode::CREATED, Json(to_wire(share))).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `DELETE /api/engagement/{doctype}/{name}/shares/{user}`
pub async fn remove_share<S>(
    State(state): State<Arc<ApiState>>,
    _user: AuthenticatedUser,
    Path((doctype, name, shared_with)): Path<(String, String, String)>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Some(response) = guard_share(&state.pool, &doctype, &name).await {
        return response;
    }
    let manager = match share_manager(&state).await {
        Ok(manager) => manager,
        Err(e) => return crate::router::error_response(e),
    };
    match manager.unshare(&doctype, &name, &shared_with).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// Share request body.
#[derive(Debug, Default, Deserialize)]
pub struct ShareBody {
    #[serde(default)]
    user: String,
    #[serde(default)]
    read: bool,
    #[serde(default)]
    write: bool,
    #[serde(default)]
    share: bool,
}

/// Map the store's `chrono`-bearing [`DocShare`] to the string-timestamped wire
/// [`Share`], so the contract module stays free of the date stack.
fn to_wire(share: DocShare) -> Share {
    Share {
        name: share.name,
        doctype: share.doctype,
        docname: share.docname,
        user: share.user,
        read: share.read,
        write: share.write,
        share: share.share,
        everyone: share.everyone,
        created_at: share.created_at.to_rfc3339(),
        modified_at: share.modified_at.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ensure_writable;
    use ferris_core::{AuthzError, Error, value};
    use ferris_document::{DocumentEngine, EngineContext};

    fn local_user(id: &str) -> ferris_auth::User {
        let mut user = ferris_auth::User::system_user(id.to_string(), id.to_string());
        user.id = id.to_string();
        user
    }

    /// The mapping must carry every flag through, and the timestamps must be
    /// RFC 3339 so the Desk can parse them with the same code it uses for the
    /// other side tables.
    #[test]
    fn doc_share_maps_to_the_wire_share() {
        let mut doc = DocShare::new("ToDo".into(), "TD-1".into(), "grace@example.com".into());
        doc.set_permission(SharePermission::Read, true);
        doc.set_permission(SharePermission::Write, true);

        let wire = to_wire(doc);
        assert_eq!(wire.doctype, "ToDo");
        assert_eq!(wire.docname, "TD-1");
        assert_eq!(wire.user, "grace@example.com");
        assert!(wire.read);
        assert!(wire.write);
        assert!(!wire.share);
        assert!(chrono::DateTime::parse_from_rfc3339(&wire.created_at).is_ok());
        assert!(chrono::DateTime::parse_from_rfc3339(&wire.modified_at).is_ok());
    }

    /// A body with only the user names a real grant: sharing with nobody
    /// allowed to do anything is not a share.
    #[test]
    fn share_body_defaults_are_readable_from_json() {
        let body: ShareBody =
            serde_json::from_value(serde_json::json!({ "user": "grace@example.com" }))
                .expect("body parses");
        assert_eq!(body.user, "grace@example.com");
        assert!(!body.read && !body.write && !body.share);
    }

    /// Assigning, attaching or sharing is a write, so an authorized caller may
    /// do it and a stranger may not. Without this gate the side tables would be
    /// a way to mutate a document the caller can only see.
    #[tokio::test]
    async fn ensure_writable_allows_an_authorized_caller_and_refuses_a_stranger() {
        let pool = ferris_db::memory_db().await.unwrap();
        let mut admin = local_user("admin@local");
        admin.roles = vec!["System Manager".to_string()];
        let admin_ctx = {
            let mut ctx = EngineContext::new("Probe", "PRO-.#####");
            ctx.allow_explicit_name = true;
            ctx.user = Some(admin.clone());
            ctx
        };
        ferris_document::with_user(Some(admin.clone()), async {
            DocumentEngine::new(pool.clone())
                .create_owned_by(&admin_ctx, "bob@local", value!({ "name": "P-1" }))
                .await
                .unwrap();
        })
        .await;

        ferris_document::with_user(Some(admin), async {
            ensure_writable(&pool, "Probe", "P-1")
                .await
                .expect("an authorized caller may write the document");
        })
        .await;

        ferris_document::with_user(Some(local_user("mallory@local")), async {
            let err = ensure_writable(&pool, "Probe", "P-1").await.unwrap_err();
            assert!(
                matches!(err, Error::Authz(AuthzError::PermissionDenied { .. })),
                "a stranger must not write a document they cannot see: {err:?}"
            );
        })
        .await;
    }
}
