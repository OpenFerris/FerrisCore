//! Dashboards that live in documents, merged into the Desk UI feed.
//!
//! A dashboard has two possible homes:
//!
//! - a plugin **declares** one in `describe()`, and the registry serves it
//!   (`ferris_core::ui_extensions`), or
//! - an administrator **creates** a `Dashboard` document, and this module serves
//!   it — the point of the `DocType`: the definition is editable data, with a
//!   child table of widget placements, not a compiled-in view.
//!
//! Both reach the Desk through the same feed and the same shape. [`db_app`]
//! produces one synthetic [`contract::AppUi`] from every `Dashboard` row and
//! [`crate::ui::ui_extensions`] appends it after the registry's sets, so:
//!
//! - the Desk renders document dashboards through the identical 12-column grid
//!   (it resolves a dashboard by `(owner, id)` and route, whoever declared it),
//! - the feed's `revision` changes when a document changes, so the Desk refetches
//!   exactly as it does after a plugin hot reload,
//! - `GET /api/dashboard-layout/db/{id}` answers the per-user override and the
//!   declared seed, because `dashboard_layout::seed_widgets` falls back to
//!   [`dashboard_widgets`] for this owner.
//!
//! The rows are read as the **service principal** ([`crate::service`]): a
//! dashboard definition is site-wide UI, not one user's data, so it is not
//! filtered by the caller's permissions — the same reason a plugin dashboard,
//! which the registry carries globally, is not.
//!
//! The registry's own route rules are reused here
//! ([`ferris_core::ui_extensions::dashboard_route_problem`], one default, unique
//! routes) rather than re-implemented, so a hand-written document cannot
//! register a route the registry would refuse.
//!
//! # Failure policy
//!
//! A `Dashboard` read is *additive* to the feed: if the table or a row cannot be
//! read, the registry's dashboards still serve and the Desk still works. The
//! failure is logged at `warn` and the document set is treated as empty — never
//! silent, and never fatal to an endpoint the whole Desk loads on every page.

use std::collections::HashSet;

use ferris_core::ui_extensions::dashboard_route_problem;
use ferris_core::{DocumentError, Error, Value};
use ferris_db::Db;
use ferris_document::{DocumentEngine, ListQuery};
use ferris_schema::contract;

use crate::service::service_context;

mod convert;

use convert::{instances, to_dashboard};

/// The `DocType` a dashboard definition is authored as.
pub const DASHBOARD: &str = "Dashboard";
/// The child table carrying one dashboard's widget placements.
pub const DASHBOARD_WIDGET: &str = "Dashboard Widget";
/// The feed owner document-declared dashboards are served under.
///
/// [`contract::AppUi::owner`] names the plugin that declared a set; these
/// documents have no plugin behind them, so they ride under one synthetic owner.
/// The Desk keys a dashboard by `(owner, id)`, so this is the `owner` in
/// `GET /api/dashboard-layout/db/{id}`.
///
/// One constant, not two: the Desk reads [`contract::NATIVE_DASHBOARD_OWNER`]
/// to open a `Dashboard` document by name, and it has to agree with the owner
/// this module publishes under or the two would never meet.
pub const OWNER: &str = contract::NATIVE_DASHBOARD_OWNER;
/// Purpose id of the service principal that reads the rows.
const SERVICE_ID: &str = "dashboards@ferris.local";

/// How many dashboard documents one feed serves.
///
/// A site with more than this many dashboards is far past the point where one
/// grid is useful; the cap keeps the feed — which the Desk loads on every page —
/// from growing with the table.
const MAX_DASHBOARDS: u32 = 500;

/// The `Dashboard` documents as one feed [`contract::AppUi`], or `None` when
/// there is none.
///
/// `None` (rather than an empty app) is the normal case for an install that
/// never uses the `DocType`, and keeps the feed identical to a registry-only
/// install.
pub async fn db_app(pool: &Db) -> Option<contract::AppUi> {
    let dashboards = list(pool).await;
    if dashboards.is_empty() {
        return None;
    }
    Some(contract::AppUi {
        owner: OWNER.to_string(),
        dashboards,
        ..contract::AppUi::default()
    })
}

/// Every `Dashboard` document as a feed dashboard, in document-name order.
///
/// Rows that could not be registered — an unusable route, a route already taken
/// by a plugin or an earlier document, a missing name — are dropped with a
/// `warn`, mirroring `collect_dashboards` in `ferris-core` so the feed never
/// carries a dashboard the registry rules would have refused.
pub async fn list(pool: &Db) -> Vec<contract::Dashboard> {
    let rows = read_rows(pool).await;
    let dashboards: Vec<contract::Dashboard> = rows.iter().filter_map(to_dashboard).collect();

    // The registry's sets are validated already; seed the taken set from them so
    // a document cannot shadow a plugin's route, and follow the same
    // "first registered wins" rule for the default dashboard.
    let apps = ferris_core::ui_extensions::registered_ui();
    let taken_routes: HashSet<String> = apps
        .iter()
        .flat_map(|app| app.dashboards.iter().map(|d| d.route.clone()))
        .collect();
    let default_taken = apps
        .iter()
        .any(|app| app.dashboards.iter().any(|d| d.default));

    apply_registry_rules(dashboards, taken_routes, default_taken)
}

/// Register a batch of document dashboards under the same rules the registry
/// applies to plugin dashboards.
///
/// A route that is unusable ([`dashboard_route_problem`]) or already taken — by
/// a plugin or by an earlier document in the batch — drops the dashboard; a
/// second `default` is cleared rather than dropping the dashboard. Kept in the
/// input order, which [`list`] fixes to document-name order.
///
/// Split out from [`list`] so the rules are testable without the process-global
/// registry, whose contents other tests mutate.
fn apply_registry_rules(
    dashboards: Vec<contract::Dashboard>,
    mut taken_routes: HashSet<String>,
    mut default_taken: bool,
) -> Vec<contract::Dashboard> {
    let mut kept = Vec::with_capacity(dashboards.len());
    for mut dashboard in dashboards {
        if let Some(problem) = dashboard_route_problem(&dashboard.route) {
            tracing::warn!(
                dashboard = %dashboard.id,
                route = %dashboard.route,
                problem,
                "Dashboard document dropped from the UI feed"
            );
            continue;
        }
        if !taken_routes.insert(dashboard.route.clone()) {
            tracing::warn!(
                dashboard = %dashboard.id,
                route = %dashboard.route,
                "Dashboard document dropped: its route is already taken"
            );
            continue;
        }
        if dashboard.default && default_taken {
            tracing::warn!(
                dashboard = %dashboard.id,
                "Dashboard document: another default dashboard is already registered — `is_default` ignored"
            );
            dashboard.default = false;
        }
        if dashboard.default {
            default_taken = true;
        }
        kept.push(dashboard);
    }
    kept
}

/// The widget instances declared by the `Dashboard` document named `id`, or
/// `None` when no such document exists.
///
/// This is the seed `dashboard_layout::seed_widgets` falls back to for
/// [`OWNER`]: without it, `GET`/`DELETE /api/dashboard-layout/db/{id}` would
/// answer an empty seed and the Desk would wipe a document dashboard's widgets
/// on "Restaurar".
pub async fn dashboard_widgets(pool: &Db, id: &str) -> Option<Vec<contract::WidgetInstance>> {
    let meta = ferris_schema::get_doctype(DASHBOARD);
    let ctx = service_context(DASHBOARD, meta.as_deref(), SERVICE_ID, "Dashboards");
    let engine = DocumentEngine::new(pool.clone());
    match engine.get(&ctx, id).await {
        Ok(document) => Some(instances(document.get("widgets"))),
        // A dashboard the feed never served has no seed; that is not an error.
        Err(Error::Document(DocumentError::NotFound { .. })) => None,
        Err(error) => {
            tracing::warn!(%error, dashboard = %id, "reading a Dashboard document failed");
            None
        }
    }
}

/// Read `Dashboard` documents with the fields the feed needs.
///
/// A read failure is logged and treated as "no document dashboards", so the
/// endpoint degrades to the registry sets instead of failing (see the module
/// docs' failure policy).
async fn read_rows(pool: &Db) -> Vec<Value> {
    let meta = ferris_schema::get_doctype(DASHBOARD);
    let ctx = service_context(DASHBOARD, meta.as_deref(), SERVICE_ID, "Dashboards");
    let engine = DocumentEngine::new(pool.clone());
    let query = ListQuery {
        fields: Some(vec![
            "name".to_string(),
            "label".to_string(),
            "route".to_string(),
            "is_default".to_string(),
            "layout".to_string(),
            "widgets".to_string(),
        ]),
        order_by: Some("name asc".to_string()),
        limit: Some(MAX_DASHBOARDS),
        ..ListQuery::default()
    };
    match engine.list(&ctx, &query).await {
        Ok((rows, _total)) => rows,
        Err(error) => {
            tracing::warn!(
                %error,
                "reading Dashboard documents failed; serving registry dashboards only"
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests;
