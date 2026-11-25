use std::sync::Arc;

use axum::{
    Router,
    extract::FromRef,
    middleware::from_fn_with_state,
    routing::{delete, get, post, put},
};
use ferris_auth::AuthState;
use ferris_auth::middleware::require_csrf;
use ferris_core::FeatureFlags;

use super::ApiState;
use super::doctype::{
    connections, doc_actions, meta, permission_matrix, permission_types, set_custom_permission,
};
use super::doctype_admin::{create as create_doctype, update as update_doctype};
use super::methods::{batch, call_method, jsonrpc, openapi};
use super::resource::{
    amend, cancel, create, get_one, get_single, list, list_requested_authed, neighbors, remove,
    rename, restore, save_single, submit, update,
};
use super::role_permissions::{
    delete_role_permission, list_role_permissions, upsert_role_permission,
};
use super::widget_assets::{widget_script, widget_style};
use crate::data::{bulk_update, export, import};
use crate::module::{Module, ModuleState};

// One route table. A second, smaller table used to live here
// (`api_router`/`api_router_with_state`, 20 routes) built for a bare
// `Arc<ApiState>`. It had no production caller and had already drifted from
// the authenticated table — it served `list_requested` unauthenticated and was
// missing the whole auth/panel/desired-state/engagement/MFA surface and the
// CSRF layer. Two implementations of one policy is exactly the failure
// `AGENTS.md` §4 forbids, so the table was removed: there is now a single
// `api_routes`, reached only through [`api_router_with_app_state`].

/// Build the API router with a parent state that provides `ApiState` via `FromRef`.
/// This allows embedding the API router in a larger application state.
///
/// The router carries its **internal** CSRF layer: for requests
/// authenticated by session cookie, every mutating route demands the session's
/// `X-CSRF-Token` header; API-key callers and `GET`/`HEAD`/`OPTIONS` are
/// exempt. The gate sits *inside* `require_auth` (which callers layer on top),
/// so it can use `AuthState` without re-authenticating, and the public auth
/// routes (`login`/`verify_mfa`) stay outside it on purpose.
pub fn api_router_with_app_state<S>(state: S) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    Arc<ApiState>: FromRef<S>,
    AuthState: FromRef<S>,
{
    api_router_with_modules(state, &[])
}

/// The API router plus the native modules the composition root registered.
///
/// Each module contributes a `Router<ModuleState>` with routes relative to its
/// `mount()` prefix. The host injects the module state once and nests the
/// resulting service **inside** the CSRF layer, so a vertical gets the same
/// authentication, CSRF and body policy as the generic REST endpoints. A module
/// that is not in `modules` has no routes at all: the answer is a plain 404,
/// never a handler that warns or pretends to work.
pub fn api_router_with_modules<S>(state: S, modules: &[Arc<dyn Module>]) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    Arc<ApiState>: FromRef<S>,
    AuthState: FromRef<S>,
{
    // Internal CSRF layer. `AuthState::from_ref(&state)` gives the
    // layer its own copy of the auth state, so it can live here without
    // disturbing the parent router's state resolution.
    // The flags live on the API state; the route table is built from them, so
    // a disabled framework route is absent rather than a handler returning a
    // warning.
    let features = Arc::<ApiState>::from_ref(&state).features.clone();
    let module_state = ModuleState::from_state(&state);
    let mut router = api_routes::<S>(&features);

    // One nested service per mount prefix. Modules that share `/api` merge
    // into a single router first: axum allows one wildcard service per prefix,
    // and merging also turns a duplicate path into a boot-time error instead
    // of one module silently shadowing another.
    let mut mounts: Vec<(&'static str, Router<ModuleState>)> = Vec::new();
    for module in modules {
        if let Some((_, mounted)) = mounts
            .iter_mut()
            .find(|(prefix, _)| *prefix == module.mount())
        {
            let existing = std::mem::replace(mounted, Router::new());
            *mounted = existing.merge(module.routes());
        } else {
            mounts.push((module.mount(), module.routes()));
        }
    }
    for (prefix, mounted) in mounts {
        router = router.nest_service(prefix, mounted.with_state(module_state.clone()));
    }

    router
        .layer(from_fn_with_state(
            AuthState::from_ref(&state),
            require_csrf,
        ))
        .with_state(state)
}

/// The one and only API route table, before the outer state and
/// the internal CSRF layer are applied.
///
/// A caller that wants the API must supply a state from which both
/// `Arc<ApiState>` and `AuthState` can be extracted, so there is no second,
/// smaller table that can silently fall out of date with this one.
fn api_routes<S>(features: &FeatureFlags) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    Arc<ApiState>: FromRef<S>,
    AuthState: FromRef<S>,
{
    let mut router = Router::new()
        .route("/api/resource/{doctype}", get(list).post(create))
        .route("/api/resource/{doctype}/list", post(list_requested_authed))
        .route(
            "/api/resource/{doctype}/{name}",
            get(get_one).put(update).delete(remove),
        )
        .route("/api/resource/{doctype}/{name}/neighbors", get(neighbors))
        .route("/api/resource/{doctype}/{name}/submit", post(submit))
        .route("/api/resource/{doctype}/{name}/cancel", post(cancel))
        .route("/api/resource/{doctype}/{name}/amend", post(amend))
        .route("/api/resource/{doctype}/{name}/rename", post(rename))
        .route("/api/resource/{doctype}/restore/{token}", post(restore))
        // Bulk data entry points (Frappe's Data Import/Export/Bulk Update).
        // They run on the same `DocStore` as `/api/resource`, so permissions and
        // validation are identical; only the batching and the file framing are
        // new. Kept next to the resource routes on purpose: this is the generic
        // store's own surface, not a vertical.
        .route("/api/data/import/{doctype}", post(import))
        .route("/api/data/export/{doctype}", get(export))
        .route("/api/data/bulk-update/{doctype}", post(bulk_update))
        .route("/api/single/{doctype}", get(get_single).put(save_single))
        .route("/api/doctype", post(create_doctype))
        .route("/api/doctype/{name}", put(update_doctype))
        .route("/api/doctype/meta/{doctype}", get(meta))
        .route("/api/doctype/actions/{doctype}", get(doc_actions))
        .route(
            "/api/doctype/form-tours/{doctype}",
            get(crate::form_tours::list),
        )
        .route(
            "/api/doctype/connections/{doctype}/{name}",
            get(connections),
        )
        .route("/api/doctype/permissions/{doctype}", get(permission_types))
        .route(
            "/api/doctype/permissions/{doctype}/manage",
            get(permission_matrix).put(set_custom_permission),
        )
        .route("/api/method/{method}", post(call_method))
        .route("/api/jsonrpc", post(jsonrpc))
        .route("/api/batch", post(batch))
        .route("/api/openapi.json", get(openapi))
        .route("/api/graphql", post(crate::graphql::graphql_handler))
        .route("/api/modules", get(crate::desk::modules))
        .route("/api/site-settings", get(crate::site::site_settings))
        .route("/api/themes", get(crate::themes::themes))
        .route("/api/ui-extensions", get(crate::ui::ui_extensions))
        // The `Widget` DocType is the source of truth for widget code; the Desk
        // loads these two URLs only when it mounts a widget whose row has no
        // original `code_path` (a self-contained, user-created widget).
        .route("/api/widgets/{app}/{name}/script.js", get(widget_script))
        .route("/api/widgets/{app}/{name}/style.css", get(widget_style))
        // R-FW-1b: persistent per-entity desired state, published by the control
        // plane and pulled by edge agents. The pull half is a conditional GET
        // (`If-None-Match` → 304) so a tight poll stays cheap.
        .route("/api/desired-state", get(crate::desired_state::list_all))
        .route(
            "/api/desired-state/{entity}",
            get(crate::desired_state::pull_one)
                .put(crate::desired_state::publish_one)
                .delete(crate::desired_state::clear_one),
        )
        .route("/api/notifications", get(crate::panel::notifications))
        .route(
            "/api/notifications/stream",
            get(crate::notifications::notifications_stream),
        )
        .route(
            "/api/notifications/read",
            post(crate::panel::mark_notifications_read),
        )
        .route(
            "/api/notifications/{name}/read",
            post(crate::panel::mark_notification_read),
        )
        .route(
            "/api/notifications/{name}/dismiss",
            post(crate::panel::dismiss_notification),
        )
        .route("/api/events", get(crate::panel::events))
        .route(
            "/api/notification-settings",
            get(crate::panel::get_notification_settings)
                .put(crate::panel::put_notification_settings),
        )
        .route(
            "/api/desk-layout",
            get(crate::panel::get_desk_layout).put(crate::panel::put_desk_layout),
        )
        .route(
            "/api/dashboard-layout/{owner}/{dashboard}",
            get(crate::dashboard_layout::get_dashboard_layout)
                .put(crate::dashboard_layout::put_dashboard_layout)
                .delete(crate::dashboard_layout::delete_dashboard_layout),
        )
        // The document-format sibling: one segment deeper than the pair route
        // above, so `doc` never collides with `{owner}` — a document that *is*
        // its own dashboard has no `(owner, id)` pair and no `DELETE` (its
        // fields are the state, nothing to restore).
        .route(
            "/api/dashboard-layout/doc/{doctype}/{name}",
            get(crate::dashboard_layout_doc::get_document_layout)
                .put(crate::dashboard_layout_doc::put_document_layout),
        )
        // The global board style: one row shared by the install, the base every
        // dashboard layers its declared style and saved override over.
        .route(
            "/api/dashboard-settings",
            get(crate::dashboard_settings::get_dashboard_settings)
                .put(crate::dashboard_settings::put_dashboard_settings),
        )
        .route(
            "/api/engagement/{doctype}",
            post(crate::engagement::engagement),
        )
        .route(
            "/api/engagement/{doctype}/{name}/like",
            post(crate::engagement::toggle_like),
        )
        .route(
            "/api/engagement/{doctype}/{name}/comments",
            get(crate::engagement::list_comments).post(crate::engagement::add_comment),
        )
        .route(
            "/api/activity/{doctype}/{name}",
            get(crate::activity::activity),
        )
        .route(
            "/api/engagement/{doctype}/{name}/assignments",
            get(crate::collab::list_assignments).post(crate::collab::add_assignment),
        )
        .route(
            "/api/engagement/{doctype}/{name}/assignments/{user}",
            delete(crate::collab::remove_assignment),
        )
        .route(
            "/api/engagement/{doctype}/{name}/attachments",
            get(crate::collab::list_attachments).post(crate::collab::add_attachment),
        )
        .route(
            "/api/engagement/{doctype}/{name}/attachments/{id}",
            delete(crate::collab::remove_attachment),
        )
        .route(
            "/api/engagement/{doctype}/{name}/shares",
            get(crate::collab::list_shares).post(crate::collab::add_share),
        )
        .route(
            "/api/engagement/{doctype}/{name}/shares/{user}",
            delete(crate::collab::remove_share),
        )
        .route(
            "/api/customization/{doctype}",
            get(crate::customization::get_customization),
        )
        .route(
            "/api/customization/{doctype}/fields",
            post(crate::customization::put_custom_field),
        )
        .route(
            "/api/customization/{doctype}/fields/{fieldname}",
            delete(crate::customization::delete_custom_field),
        )
        .route(
            "/api/customization/{doctype}/properties",
            post(crate::customization::put_property_setter),
        )
        .route(
            "/api/customization/{doctype}/properties/{property}",
            delete(crate::customization::delete_property_setter),
        )
        .route(
            "/api/customization/{doctype}/export",
            post(crate::customization::export),
        )
        // Client error reporting from the Desk's form UI. A log sink only: it
        // stores nothing and echoes nothing, so it needs no feature gate and no
        // DocType.
        .route("/api/log/client", post(crate::log::client_report))
        .route(
            "/api/authz/role-permissions",
            get(list_role_permissions).post(upsert_role_permission),
        )
        .route(
            "/api/authz/role-permissions/{doctype}/{role}",
            delete(delete_role_permission),
        )
        // Authenticated MFA self-service: enable/activate/disable.
        // These live inside `require_auth` and inside the CSRF gate.
        .route("/api/auth/mfa/enable", post(crate::mfa::enable))
        .route("/api/auth/mfa/activate", post(crate::mfa::activate))
        .route("/api/auth/mfa/disable", post(crate::mfa::disable));

    // `[features]`: a disabled subsystem has no route. A 404 is the honest
    // answer for an endpoint the operator turned off, and it cannot be confused
    // with "no results".
    if features.search {
        router = router.route("/api/search", get(crate::desk::search));
    }
    router
}
