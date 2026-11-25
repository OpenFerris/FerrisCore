//! `GET /api/activity/{doctype}/{name}` — one document's activity timeline.
//!
//! The feed merges the write log (`tab_version`), the comments and the likes
//! into a single time-ordered list ([`ferris_document::ActivityStore`]); this
//! module is only the HTTP surface and the read-permission gate.

use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::response::{IntoResponse, Response};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_document::ActivityStore;

use crate::ApiState;

/// `GET /api/activity/{doctype}/{name}` — every recorded event of a document,
/// oldest first.
///
/// Guarded by `Read` on the document: a document's history names its fields,
/// its actors and its discussion, so it is not public to anyone who can spell
/// the name (see `crate::store::ensure_readable`).
pub async fn activity<S>(
    State(state): State<Arc<ApiState>>,
    _user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Err(e) = crate::store::validate_doctype(&doctype) {
        return crate::router::error_response(e);
    }
    if let Err(e) = crate::store::validate_name(&name) {
        return crate::router::error_response(e);
    }
    if let Err(e) = crate::store::ensure_readable(&state.pool, &doctype, &name).await {
        return crate::router::error_response(e);
    }

    match ActivityStore::new(state.pool.clone())
        .feed(&doctype, &name)
        .await
    {
        Ok(feed) => Json(feed).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::DocStore;
    use crate::test_support::register_probe;
    use ferris_core::{AuthzError, Error, value};
    use ferris_document::{ActivityKind, EngineContext};

    fn admin() -> ferris_auth::User {
        let mut user =
            ferris_auth::User::system_user("admin@local".to_string(), "Admin".to_string());
        user.id = "admin@local".to_string();
        user.roles = vec!["System Manager".to_string()];
        user
    }

    /// The whole point of the wiring: REST writes show up in the feed, in order,
    /// with the changed field named and *only* the changed field (the engine's
    /// own `modified`/`modified_by` must not appear as user edits).
    #[tokio::test]
    async fn rest_writes_are_recorded_in_the_activity_feed() {
        register_probe();
        let pool = ferris_db::memory_db().await.unwrap();
        ferris_document::with_user(Some(admin()), async {
            DocStore::create(
                &pool,
                "Probe",
                value!({ "name": "P-1", "title": "hello", "qty": 1 }),
            )
            .await
            .unwrap();
            DocStore::update(&pool, "Probe", "P-1", value!({ "title": "bye" }))
                .await
                .unwrap();

            let feed = ActivityStore::new(pool.clone())
                .feed("Probe", "P-1")
                .await
                .unwrap();
            let kinds: Vec<ActivityKind> = feed.iter().map(|e| e.kind).collect();
            assert_eq!(kinds, vec![ActivityKind::Create, ActivityKind::Update]);
            assert_eq!(feed[1].fields, vec!["title".to_string()]);
            assert_eq!(feed[0].actor, "admin@local");
        })
        .await;
    }

    /// `ensure_readable` is the gate that keeps the side-table endpoints from
    /// handing one user another user's history: a document Bob owns is not
    /// readable by Mallory, so her feed request is refused at the engine — the
    /// same single permission policy the resource routes use.
    #[tokio::test]
    async fn ensure_readable_refuses_a_document_the_caller_cannot_read() {
        let pool = ferris_db::memory_db().await.unwrap();
        let admin_ctx = {
            let mut ctx = EngineContext::new("Probe", "PRO-.#####");
            ctx.allow_explicit_name = true;
            ctx.user = Some(admin());
            ctx
        };
        ferris_document::with_user(Some(admin()), async {
            ferris_document::DocumentEngine::new(pool.clone())
                .create_owned_by(&admin_ctx, "bob@local", value!({ "name": "SECRET-1" }))
                .await
                .unwrap();
        })
        .await;

        let mut mallory =
            ferris_auth::User::system_user("mallory@local".to_string(), "M".to_string());
        mallory.id = "mallory@local".to_string();
        ferris_document::with_user(Some(mallory), async {
            let err = crate::store::ensure_readable(&pool, "Probe", "SECRET-1")
                .await
                .unwrap_err();
            assert!(
                matches!(err, Error::Authz(AuthzError::PermissionDenied { .. })),
                "the history of an unreadable document must be refused: {err:?}"
            );
        })
        .await;
    }
}
