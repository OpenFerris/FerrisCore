//! The `Widget` `DocType`: the runtime source of truth for the widget catalog.
//!
//! Before this module the catalog existed only in the process-wide registry:
//! `describe().widgets` was read once at registration and every later change
//! required a plugin rebuild and a hot reload. The `Widget` `DocType` moves that
//! definition into data — metadata, params and the widget's own `JavaScript` and
//! CSS (with optional SSR markup kept alongside) — so an administrator can edit a
//! widget from the Desk and the next feed/mount uses the stored module and
//! stylesheet.
//!
//! # How the two halves meet
//!
//! * [`sync`] is the **initial migration**: the registry still carries what
//!   `describe()` declared, and this function upserts each definition into the
//!   `DocType` keyed by `"{app}.{widget_name}"`. A `seed_hash` fingerprint guards
//!   the row: when the stored hash still matches the app's seed the row may
//!   carry admin edits and is left alone; when the app's definition changes the
//!   hash changes and the seed-controlled fields are updated. A row that does
//!   not exist is created.
//! * [`list_apps`] reads the rows back as feed [`contract::AppUi`]s and
//!   republishes the definitions into the registry with
//!   [`ferris_core::ui_extensions::replace_widgets`], so the synchronous layout
//!   helpers (`find_widgets`, dashboard normalization) see exactly what the
//!   database says.
//!
//! Runtime code is served from the row, never from the app's asset manifest:
//! [`script_by_name`]/[`stylesheet_by_name`] answer the canonical
//! `/api/widgets/...` URLs, and [`script_by_path`] lets the plugin asset route
//! keep serving a widget at its original `code_path` so relative `import`s
//! inside the module still resolve against the app's assets.
//!
//! # Failure policy
//!
//! A `Widget` read is additive to the feed, like document dashboards: a failure
//! is logged at `warn` and the registry's seed keeps serving, so one bad table
//! cannot take down the Desk. The seed itself never overwrites a row whose
//! hash matches, which is what makes "apps migrate once, the Desk owns it"
//! safe on every boot.

use std::collections::{BTreeMap, HashMap, HashSet};

use ferris_core::ui_extensions::{self as registry, WidgetDefSpec};
use ferris_core::{DocumentError, Error, Value};
use ferris_db::Db;
use ferris_document::{DocumentEngine, ListQuery};
use ferris_schema::contract;

use crate::service::service_context;

mod model;
#[cfg(test)]
mod tests;

use model::{stored_from_value, to_contract, to_value};

/// The `DocType` a widget definition is stored as.
pub const WIDGET: &str = "Widget";
/// The child table carrying one widget's parameters.
pub const WIDGET_PARAM: &str = "Widget Param";
/// Purpose id of the service principal that seeds and reads the rows.
const SERVICE_ID: &str = "widgets@ferris.local";
/// Cap on one feed's widget set. A catalog larger than this is past the point
/// where the palette is usable; the cap keeps a pathological table from growing
/// the feed the Desk loads on every page.
const MAX_WIDGETS: u32 = 2000;
/// Every field [`Stored`] is built from. One list shape for the feed and the
/// point lookups, so a new field cannot be added to one and forgotten in the
/// other.
const ROW_FIELDS: &[&str] = &[
    "name",
    "app",
    "widget_name",
    "label",
    "component",
    "kind",
    "renderer",
    "description",
    "requires",
    "events",
    "icon",
    "category",
    "preview",
    "style",
    "span",
    "rows",
    "min_span",
    "min_rows",
    "params",
    "enabled",
    "version",
    "code_path",
    "script",
    "stylesheet",
    "ssr",
    "seed_hash",
];
/// URL prefix of the canonical per-widget code endpoint.
const API_PREFIX: &str = "/api/widgets";

/// One stored widget row, in its spec shape.
#[derive(Debug, Clone, PartialEq)]
struct Stored {
    app: String,
    def: WidgetDefSpec,
    enabled: bool,
    seed_hash: String,
}

/// The initial migration: upsert every registry definition into the `Widget`
/// `DocType`, skipping rows whose seed hash already matches.
///
/// Called by the UI feed before [`list_apps`]. The registry is the app's
/// `describe()` output; after this call the database is authoritative. A
/// failure is logged and makes the function return `false`, so the caller can
/// keep serving the registry's seed instead of publishing a table that is
/// missing the failed rows.
pub async fn sync(pool: &Db) -> bool {
    let defs: Vec<(String, WidgetDefSpec)> = registry::registered_ui()
        .into_iter()
        .flat_map(|app| {
            app.widgets
                .into_iter()
                .map(move |def| (app.owner.clone(), def))
        })
        .filter(|(app, def)| !app.trim().is_empty() && !def.name.trim().is_empty())
        .collect();
    if defs.is_empty() {
        return true;
    }

    let existing = match read_hashes(pool).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "reading Widget seed hashes failed; skipping the widget migration");
            return false;
        }
    };

    let meta = ferris_schema::get_doctype(WIDGET);
    let ctx = service_context(WIDGET, meta.as_deref(), SERVICE_ID, "Widgets");
    let engine = DocumentEngine::new(pool.clone());
    let mut complete = true;

    for (app, def) in defs {
        let document_name = document_name(&app, &def.name);
        let hash = seed_hash(&def);
        if existing
            .get(&document_name)
            .is_some_and(|stored| stored == &hash)
        {
            continue;
        }
        let create = !existing.contains_key(&document_name);
        let data = to_value(&app, &def, &hash, create);
        let result = if create {
            engine.create(&ctx, data).await
        } else {
            engine.update(&ctx, &document_name, data).await
        };
        if let Err(error) = result {
            complete = false;
            tracing::warn!(
                %error,
                app = %app,
                widget = %def.name,
                "widget migration upsert failed"
            );
        }
    }
    complete
}

/// Read every enabled `Widget` row as feed apps and refresh the registry cache.
///
/// `None` means the read itself failed; the caller keeps the registry's seed
/// rather than silently publishing an empty catalog. An empty `Some` is a real
/// answer (no rows yet).
pub async fn list_apps(pool: &Db) -> Option<Vec<contract::AppUi>> {
    let rows = match read_rows(pool).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "reading Widget documents failed; serving registry widgets only");
            return None;
        }
    };

    let mut grouped: BTreeMap<String, Vec<Stored>> = BTreeMap::new();
    for row in &rows {
        let Some(stored) = stored_from_value(row) else {
            continue;
        };
        if stored.enabled {
            grouped.entry(stored.app.clone()).or_default().push(stored);
        }
    }

    // Clear the cache for every owner the registry knows, not only the owners
    // present in the table: disabling the last widget of an app must stop it
    // resolving in the layout helpers too.
    let owners: HashSet<String> = registry::registered_ui()
        .into_iter()
        .map(|app| app.owner)
        .chain(grouped.keys().cloned())
        .collect();
    for owner in &owners {
        let defs = grouped
            .get(owner)
            .map(|stored| stored.iter().map(|row| row.def.clone()).collect::<Vec<_>>())
            .unwrap_or_default();
        registry::replace_widgets(owner, defs);
    }

    Some(
        grouped
            .into_iter()
            .map(|(owner, mut stored)| {
                stored.sort_by(|a, b| a.def.name.cmp(&b.def.name));
                contract::AppUi {
                    owner,
                    widgets: stored.iter().map(to_contract).collect(),
                    ..contract::AppUi::default()
                }
            })
            .collect(),
    )
}

/// Seed and read the rows, then refresh the registry cache for callers that do
/// not build a feed (the layout endpoints). The boolean says whether the read
/// succeeded.
///
/// `sync` runs first for the same reason the feed runs it: a layout request can
/// be the process's first contact with the database (a direct link to a
/// dashboard), and refreshing an empty table would otherwise clear the
/// registry's seed before the rows exist.
pub async fn refresh(pool: &Db) -> bool {
    if !sync(pool).await {
        return false;
    }
    list_apps(pool).await.is_some()
}

/// The stored JavaScript of `app.widget_name`, when the row exists and is
/// enabled. Backs `GET /api/widgets/{app}/{name}/script.js`.
pub async fn script_by_name(pool: &Db, app: &str, widget: &str) -> Option<String> {
    let stored = get_stored_by_app_name(pool, app, widget).await?;
    stored
        .enabled
        .then_some(stored.def.script)
        .flatten()
        .filter(|script| !script.trim().is_empty())
}

/// The stored CSS of `app.widget_name`, when the row exists and is enabled.
pub async fn stylesheet_by_name(pool: &Db, app: &str, widget: &str) -> Option<String> {
    let stored = get_stored_by_app_name(pool, app, widget).await?;
    stored
        .enabled
        .then_some(stored.def.stylesheet)
        .flatten()
        .filter(|css| !css.trim().is_empty())
}

/// The stored JavaScript of the enabled widget whose `code_path` is `path`.
///
/// The plugin asset route asks this before the app's `ui-asset` export, so a
/// widget edited in the Desk is served at its original asset path — which is
/// what keeps the relative `import`s inside the module resolving.
pub async fn script_by_path(pool: &Db, app: &str, path: &str) -> Option<String> {
    let mut filters = HashMap::new();
    filters.insert("app".to_string(), Value::string(app.trim()));
    filters.insert("code_path".to_string(), Value::string(path));
    let stored = find_one(pool, filters).await?;
    stored
        .enabled
        .then_some(stored.def.script)
        .flatten()
        .filter(|script| !script.trim().is_empty())
}

/// One row by `app` + `widget_name`.
///
/// The canonical name is `"{app}.{widget_name}"`, but a widget created through
/// the Desk form gets an opaque document name, so the fields are the fallback.
async fn get_stored_by_app_name(pool: &Db, app: &str, widget: &str) -> Option<Stored> {
    if let Some(stored) = get_stored(pool, &document_name(app, widget)).await {
        return Some(stored);
    }
    let mut filters = HashMap::new();
    filters.insert("app".to_string(), Value::string(app.trim()));
    filters.insert("widget_name".to_string(), Value::string(widget.trim()));
    find_one(pool, filters).await
}

/// One stored row matching `filters`.
async fn find_one(pool: &Db, filters: HashMap<String, Value>) -> Option<Stored> {
    let meta = ferris_schema::get_doctype(WIDGET);
    let ctx = service_context(WIDGET, meta.as_deref(), SERVICE_ID, "Widgets");
    let engine = DocumentEngine::new(pool.clone());
    let query = ListQuery {
        filters,
        fields: Some(
            ROW_FIELDS
                .iter()
                .map(|field| (*field).to_string())
                .collect(),
        ),
        limit: Some(1),
        ..ListQuery::default()
    };
    let (rows, _total) = match engine.list(&ctx, &query).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "reading a Widget row failed");
            return None;
        }
    };
    stored_from_value(rows.first()?)
}

/// Read one stored row by document name.
async fn get_stored(pool: &Db, name: &str) -> Option<Stored> {
    let meta = ferris_schema::get_doctype(WIDGET);
    let ctx = service_context(WIDGET, meta.as_deref(), SERVICE_ID, "Widgets");
    let engine = DocumentEngine::new(pool.clone());
    match engine.get(&ctx, name).await {
        Ok(row) => stored_from_value(&row),
        Err(Error::Document(DocumentError::NotFound { .. })) => None,
        Err(error) => {
            tracing::warn!(%error, name, "reading a Widget document failed");
            None
        }
    }
}

/// The document name of a widget: `"{app}.{widget_name}"`.
fn document_name(app: &str, widget: &str) -> String {
    format!("{}.{}", app.trim(), widget.trim())
}

/// The fingerprint the migration compares. It covers every seed-controlled
/// field (including the source), so a code change in `describe()` forces an
/// update while an unchanged app leaves the row — and admin edits — alone.
fn seed_hash(def: &WidgetDefSpec) -> String {
    serde_json::to_vec(def)
        .map(|bytes| contract::revision_of(&bytes))
        .unwrap_or_default()
}

/// Every stored `(document name, seed hash)`.
// The engine's typed `Error` is the uniform failure channel; boxing it only to
// satisfy a size lint would force every caller to unwrap a box for no gain.
#[allow(clippy::result_large_err)]
async fn read_hashes(pool: &Db) -> ferris_core::Result<HashMap<String, String>> {
    let meta = ferris_schema::get_doctype(WIDGET);
    let ctx = service_context(WIDGET, meta.as_deref(), SERVICE_ID, "Widgets");
    let engine = DocumentEngine::new(pool.clone());
    let query = ListQuery {
        fields: Some(vec!["name".to_string(), "seed_hash".to_string()]),
        limit: Some(MAX_WIDGETS),
        ..ListQuery::default()
    };
    let (rows, _total) = engine.list(&ctx, &query).await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let name = row.get("name").and_then(Value::as_str)?.to_string();
            let hash = row
                .get("seed_hash")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some((name, hash))
        })
        .collect())
}

/// Every `Widget` row, in name order.
// The engine's typed `Error` is the uniform failure channel; see `read_hashes`.
#[allow(clippy::result_large_err)]
async fn read_rows(pool: &Db) -> ferris_core::Result<Vec<Value>> {
    let meta = ferris_schema::get_doctype(WIDGET);
    let ctx = service_context(WIDGET, meta.as_deref(), SERVICE_ID, "Widgets");
    let engine = DocumentEngine::new(pool.clone());
    let query = ListQuery {
        fields: Some(
            ROW_FIELDS
                .iter()
                .map(|field| (*field).to_string())
                .collect(),
        ),
        order_by: Some("name asc".to_string()),
        limit: Some(MAX_WIDGETS),
        ..ListQuery::default()
    };
    engine.list(&ctx, &query).await.map(|(rows, _total)| rows)
}
