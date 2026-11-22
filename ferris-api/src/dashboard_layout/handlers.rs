//! The three REST handlers: read the caller's effective layout, save it, drop
//! it.
//!
//! Every answer is the *effective* state (the caller's saved row or the
//! declared seed) rather than a bare acknowledgement, because a save can change
//! more than the caller asked: unresolvable widget references are dropped,
//! spans clamped, duplicate instance ids collapsed and config numbers clamped,
//! so the Desk is told what actually got stored.
//!
//! The layout is **per user** (API-10): the saved row's name carries the
//! caller's id (`layout::layout_name`), so two people arranging the same app
//! dashboard own two independent boards. A `GET` is read-only; the `Widget`
//! catalog migration runs on the feed and on the write endpoints, never on a
//! read.

use axum::{
    Json,
    extract::{FromRef, Path, State},
    response::{IntoResponse, Response},
};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::Value;
use ferris_document::DocumentEngine;
use ferris_schema::contract::{DashboardConfig, DashboardLayout, DashboardLayoutPayload};

use crate::ApiState;
use crate::core_doctypes::DASHBOARD_LAYOUT;
use crate::panel::panel_context;
use crate::store::effective_meta;

use super::access::{deletable_for, editable_for};
use super::layout::{layout_name, normalize_widgets, sanitize_widgets};
use super::seed::{dashboard_exists, seed_widgets};
use super::stored::stored_row;

/// `GET /api/dashboard-layout/{owner}/{dashboard}` — the caller's effective
/// layout of one dashboard.
///
/// The saved row when the caller has one, else the app's seed, plus the flags
/// the editor needs (`editable`, `customized`) and the board override.
///
/// A `GET` performs **no** write (API-10). It used to call
/// `widgets::refresh`, whose `sync` upserts the registry seed into the `Widget`
/// `DocType` — a read endpoint that migrated the catalog, and one that made a
/// plain navigation able to write. The migration belongs to the feed
/// (`/api/ui-extensions`) and to `ferris migrate`; this handler resolves the
/// layout against whatever the registry has, which is the seed the process
/// booted with when no feed has run yet. The write endpoints (`PUT`/`DELETE`)
/// do refresh first, because there the caller is already writing and the
/// reference filter must see the current catalog.
pub async fn get_dashboard_layout<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((owner, dashboard)): Path<(String, String)>,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    let name = layout_name(&owner, &dashboard, &user.0.id);
    let (customized, mut widgets, config) =
        match stored_row(&state.pool, &owner, &dashboard, &user.0.id).await {
            Some(row) => (true, row.widgets, row.config),
            None => (
                false,
                seed_widgets(&state.pool, &owner, &dashboard).await,
                DashboardConfig::default(),
            ),
        }; // A stored row may predate the placement rule (or have been written by an
    // older server): normalizing again is idempotent (`layout::normalize` is
    // documented and tested as such) and keeps the module's promise — `GET`
    // answers with a layout the Desk can draw as-is.
    normalize_widgets(&owner, &mut widgets);
    Json(DashboardLayout {
        editable: editable_for(&user.0, &name),
        customized,
        widgets,
        config,
    })
    .into_response()
}

/// `PUT /api/dashboard-layout/{owner}/{dashboard}` — save the caller's widgets
/// **and** board override of one dashboard.
///
/// The target is the `(owner, dashboard)` pair from the path plus the caller's
/// own identity, never the body, so a request body cannot widen the write to
/// another dashboard or overwrite another user's layout. The answer is the
/// effective saved state rather than a bare acknowledgement, because a save can
/// change it beyond what was asked (unresolvable references are dropped, spans
/// clamped, duplicate ids collapsed, config numbers clamped).
pub async fn put_dashboard_layout<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((owner, dashboard)): Path<(String, String)>,
    Json(body): Json<DashboardLayoutPayload>,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    let name = layout_name(&owner, &dashboard, &user.0.id);

    // `sanitize_widgets` resolves references through the registry; refresh it
    // from the `Widget` DocType first so a newly deleted or edited widget is
    // reflected in the save.
    crate::widgets::refresh(&state.pool).await;

    // A save targets a dashboard that exists — declared by an app or already
    // saved once — so a typo cannot silently create a row nothing ever reads.
    // The check is on the dashboard, not on the caller's own row.
    if !dashboard_exists(&state.pool, &owner, &dashboard).await
        && stored_row(&state.pool, &owner, &dashboard, &user.0.id)
            .await
            .is_none()
    {
        return crate::router::error_response(ferris_core::Error::Validation(
            ferris_core::ValidationError::Invalid(format!(
                "no dashboard named `{dashboard}` is registered by `{owner}`"
            )),
        ));
    }
    if !editable_for(&user.0, &name) {
        return crate::router::error_response(ferris_core::Error::Authz(
            ferris_core::AuthzError::PermissionDenied {
                action: "create".to_string(),
                resource: format!("{DASHBOARD_LAYOUT} {name}"),
            },
        ));
    }

    let effective = sanitize_widgets(&owner, &body.widgets);
    let layout_json = serde_json::to_string(&effective).unwrap_or_else(|_| "[]".to_string());
    // The config is clamped on the way in, exactly like the geometry: a
    // hand-crafted payload cannot store a padding or a gap the board cannot
    // draw. `None`-valued fields survive as `None`, so "inherit" is preserved.
    let config = body.config.clamped();
    let config_json = serde_json::to_string(&config).unwrap_or_else(|_| "{}".to_string());

    let engine = DocumentEngine::new(state.pool.clone());
    let meta = effective_meta(&state.pool, DASHBOARD_LAYOUT)
        .await
        .ok()
        .flatten();
    // The row is system-owned but per user (the user is part of the name), so
    // both the read that decides insert-vs-update and the write go through the
    // service principal: the row's owner column stays the service identity and
    // two saves of one user's layout update rather than collide.
    let ctx = panel_context(DASHBOARD_LAYOUT, meta.as_ref());

    let mut data = Value::from_json(serde_json::json!({
        // `owner` is the engine's reserved column (the row's owner = the
        // service principal); the app that declared the dashboard rides in
        // `app_owner` instead.
        "app_owner": owner,
        "dashboard": dashboard,
        "widgets": layout_json,
        "config": config_json,
    }));

    let save = async {
        if crate::store::document_exists(&engine, &ctx, &name).await? {
            engine.update(&ctx, &name, data.clone()).await
        } else {
            data.set("name".to_string(), Value::from(name.clone()));
            engine.create(&ctx, data).await
        }
    };
    match save.await {
        Ok(_) => Json(DashboardLayout {
            editable: true,
            customized: true,
            widgets: effective,
            config,
        })
        .into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `DELETE /api/dashboard-layout/{owner}/{dashboard}` — drop the caller's saved
/// row and answer with the app's declared seed.
///
/// The Desk swaps the layout back in one round trip. Deleting a layout the
/// caller never saved is a no-op that still answers with the seed; another
/// user's row is never touched, because the name carries the caller.
pub async fn delete_dashboard_layout<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((owner, dashboard)): Path<(String, String)>,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    let name = layout_name(&owner, &dashboard, &user.0.id);

    // The answer is the app's seed, resolved through the registry: refresh it
    // from the `Widget` DocType so a deleted widget does not reappear. This is
    // a write endpoint, so the migration is allowed to run here.
    crate::widgets::refresh(&state.pool).await;

    if !deletable_for(&user.0, &name) {
        return crate::router::error_response(ferris_core::Error::Authz(
            ferris_core::AuthzError::PermissionDenied {
                action: "delete".to_string(),
                resource: format!("{DASHBOARD_LAYOUT} {name}"),
            },
        ));
    }

    let engine = DocumentEngine::new(state.pool.clone());
    let meta = effective_meta(&state.pool, DASHBOARD_LAYOUT)
        .await
        .ok()
        .flatten();
    let ctx = panel_context(DASHBOARD_LAYOUT, meta.as_ref());
    if let Err(e) = engine.delete(&ctx, &name).await {
        // Nothing saved yet is fine — the answer is the seed either way.
        if !matches!(
            e,
            ferris_core::Error::Document(ferris_core::DocumentError::NotFound { .. })
        ) {
            return crate::router::error_response(e);
        }
    }

    Json(DashboardLayout {
        editable: true,
        customized: false,
        widgets: seed_widgets(&state.pool, &owner, &dashboard).await,
        config: DashboardConfig::default(),
    })
    .into_response()
}
