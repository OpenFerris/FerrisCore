//! `GET|PUT /api/dashboard-settings` — the Desk's **global** board style.
//!
//! Every board resolves its look in three layers (dashboard config, phase C8):
//!
//! 1. this shared style (the Desk's base),
//! 2. the dashboard's declared style (an app's `describe().dashboards[].style`
//!    or a native `Dashboard` document's `style`),
//! 3. the dashboard's saved override (`DashboardConfig`).
//!
//! The base used to be the empty [`ferris_schema::contract::DashboardStyle`]
//! constant `style::GLOBAL` in the Desk. It is now one row of the
//! `Dashboard Settings` doctype, `name = "global"`, **shared by the install**:
//! the same product decision as the per-dashboard row in
//! [`crate::dashboard_layout`] — a dashboard is furniture, editing it edits it
//! for everyone — applied to the furniture's surface.
//!
//! ## Why a separate doctype
//!
//! A global style is not a dashboard layout: it has no widgets, no
//! `(owner, dashboard)` key and no seed to restore. Reusing `Dashboard Layout`
//! with a sentinel `app_owner` would make every layout query carry a name that
//! is not a dashboard, and would put the global style inside the table whose
//! `customized`/`DELETE` semantics mean "this board". A singleton row of its own
//! keeps both meanings exact, and gives the write its own permission rule:
//! `Create` on `Dashboard Settings`, defaulting to System Manager, because the
//! value repaints every dashboard in the install.
//!
//! ## Storage and failures
//!
//! `config` is a `Code` field holding the serialised [`DashboardConfig`] — the
//! same shape a `Dashboard Layout` row stores, so the Desk's precedence rule
//! reads both with one type. A missing row and a malformed one are the same
//! thing here: the built-in default style. That mirrors the layout endpoint's
//! stance ("a bad row is not a reason to fail a page") and means an install
//! that never set the global keeps the look it had before this feature existed.
//!
//! Writes are clamped at the boundary exactly like a per-dashboard override, so
//! a hand-crafted payload cannot store a gap or padding the board cannot draw.

use axum::{
    Json,
    extract::{FromRef, State},
    response::{IntoResponse, Response},
};

use ferris_auth::Permission;
use ferris_auth::middleware::AuthenticatedUser;
use ferris_auth::permissions::{Resource, check_permission};
use ferris_core::Value;
use ferris_db::Db;
use ferris_document::DocumentEngine;

use ferris_schema::contract::{DashboardConfig, DashboardSettings, DashboardSettingsPayload};

use crate::ApiState;
use crate::doctypes::DASHBOARD_SETTINGS;
use crate::panel::panel_context;
use crate::store::{document_exists, effective_meta};

/// The name of the one row: the global settings are a singleton, and the name
/// is what the write upserts and the read looks up.
const GLOBAL: &str = "global";

/// Whether `user` may write the global style: the `Create` grant on
/// `Dashboard Settings`, evaluated for the caller.
///
/// The read and the write both run through the service principal (the row is
/// system-owned and shared), so this explicit check is the only gate that runs
/// — the same reason [`crate::dashboard_layout`] checks it for a save.
fn editable_for(user: &ferris_auth::User) -> bool {
    check_permission(
        user,
        &Resource::new(DASHBOARD_SETTINGS, GLOBAL),
        Permission::Create,
    )
}

/// The stored global style, or the built-in default when the row is absent or
/// cannot be read.
///
/// A malformed `config` is deliberately *not* an error: the Desk would then
/// have to choose between failing every dashboard and rendering without the
/// global, and the second is what every other feed-driven value does.
async fn stored_style(pool: &Db) -> DashboardConfig {
    let engine = DocumentEngine::new(pool.clone());
    let meta = effective_meta(pool, DASHBOARD_SETTINGS)
        .await
        .ok()
        .flatten();
    let Some(doc) = engine
        .get(&panel_context(DASHBOARD_SETTINGS, meta.as_ref()), GLOBAL)
        .await
        .ok()
    else {
        return DashboardConfig::default();
    };
    doc.get("config")
        .and_then(|raw| raw.as_str())
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default()
}

/// `GET /api/dashboard-settings` — the global style and whether the caller may
/// edit it.
///
/// `editable` is answered, not guessed: the Desk hides the global scope of its
/// configuration drawer for a user who would get a `403` on save.
pub async fn get_dashboard_settings<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    user: AuthenticatedUser,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    let config = stored_style(&state.pool).await;
    Json(DashboardSettings {
        style: config.style,
        editable: editable_for(&user.0),
    })
    .into_response()
}

/// `PUT /api/dashboard-settings` — store the global style.
///
/// The answer is what actually got saved (numbers clamped), so the Desk can
/// repaint with exactly what the next load will reproduce. An empty style is a
/// valid write: it resets the global to the built-in defaults, which is what
/// "Restaurar" does.
pub async fn put_dashboard_settings<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    user: AuthenticatedUser,
    Json(body): Json<DashboardSettingsPayload>,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if !editable_for(&user.0) {
        return crate::router::error_response(ferris_core::Error::Authz(
            ferris_core::AuthzError::PermissionDenied {
                action: "create".to_string(),
                resource: format!("{DASHBOARD_SETTINGS} {GLOBAL}"),
            },
        ));
    }

    let style = body.style.clamped();
    // Stored as the same `DashboardConfig` shape a board override uses, with no
    // `dashboard_id`: this row is not a board.
    let config = DashboardConfig {
        dashboard_id: None,
        style: style.clone(),
    };
    let config_json = serde_json::to_string(&config).unwrap_or_else(|_| "{}".to_string());

    let engine = DocumentEngine::new(state.pool.clone());
    let meta = effective_meta(&state.pool, DASHBOARD_SETTINGS)
        .await
        .ok()
        .flatten();
    let ctx = panel_context(DASHBOARD_SETTINGS, meta.as_ref());
    let mut data = Value::from_json(serde_json::json!({ "config": config_json }));

    let save = async {
        if document_exists(&engine, &ctx, GLOBAL).await? {
            engine.update(&ctx, GLOBAL, data.clone()).await
        } else {
            data.set("name".to_string(), Value::from(GLOBAL));
            engine.create(&ctx, data).await
        }
    };
    match save.await {
        Ok(_) => Json(DashboardSettings {
            style,
            editable: true,
        })
        .into_response(),
        Err(e) => crate::router::error_response(e),
    }
}
