//! Tests for the document-dashboard merge.
//!
//! The pure tests drive [`to_dashboard`] and [`apply_registry_rules`] directly:
//! the registry is process-global and other tests in this binary register
//! plugin sets into it, so the rules are exercised with *controlled* inputs
//! rather than against whatever `registered_ui()` happens to hold. The database
//! tests only assert on the rows they inserted.

use std::collections::HashSet;

use ferris_core::Value;
use ferris_db::Db;
use ferris_document::DocumentEngine;
use ferris_schema::contract::Dashboard;

use super::*;

/// A pool with the `Dashboard` declarations registered.
///
/// Only the two doctypes this module reads are registered — **not**
/// `ferris_schema::doctypes::register_doctypes()`. The schema registry is
/// process-global and shared by every test in this binary, so registering the
/// whole schema would change how unrelated tests validate (e.g. a `Workspace`
/// fixture that omits its required `title`); production registers the full set
/// in `ferris-server::catalog`.
async fn pool() -> Db {
    ferris_schema::register_doctype(ferris_schema::doctypes::dashboard_meta());
    ferris_schema::register_doctype(ferris_schema::doctypes::dashboard_widget_meta());
    ferris_db::memory_db().await.unwrap()
}

/// Insert a `Dashboard` document the way the framework would — as the service
/// principal, with the `widgets` child rows already flattened into the parent's
/// `data` blob, bypassing the Desk form (which cannot edit child tables).
async fn insert(pool: &Db, name: &str, route: &str, default: bool, widgets: Vec<Value>) {
    let meta = ferris_schema::get_doctype(DASHBOARD);
    let ctx = crate::service::service_context(
        DASHBOARD,
        meta.as_deref(),
        "dashboards@ferris.local",
        "Dashboards",
    );
    let engine = DocumentEngine::new(pool.clone());
    let data = Value::from_json(serde_json::json!({
        "name": name,
        "label": name,
        "route": route,
        "is_default": default,
        "widgets": widgets,
    }));
    engine.create(&ctx, data).await.expect("dashboard inserted");
}

/// A `widgets` child row as the engine stores one.
fn widget_row(id: &str, widget: &str, span: u8, col: u8, row: u8) -> Value {
    Value::from_json(serde_json::json!({
        "widget_id": id, "widget": widget, "span": span, "col": col, "row": row,
    }))
}

fn dashboard(id: &str, route: &str, default: bool) -> Dashboard {
    Dashboard {
        id: id.to_string(),
        route: route.to_string(),
        default,
        ..Dashboard::default()
    }
}

// --- declaration -----------------------------------------------------------

/// The named field of a declaration, or a panic naming the doctype — a missing
/// field is the failure this test exists to catch.
fn field<'a>(meta: &'a ferris_schema::DoctypeMeta, name: &str) -> &'a ferris_schema::FieldMeta {
    meta.fields
        .iter()
        .find(|field| field.fieldname == name)
        .unwrap_or_else(|| panic!("`{name}` is not declared on `{}`", meta.name))
}

/// The feed reads exactly these fields, so the declaration has to carry them
/// with the types the conversion expects — otherwise the reader silently sees
/// `None` for every row and the Desk renders empty dashboards.
#[test]
fn the_dashboard_doctypes_declare_what_the_feed_reads() {
    let meta = ferris_schema::doctypes::dashboard_meta();
    assert_eq!(meta.name, DASHBOARD);

    assert_eq!(field(&meta, "label").fieldtype, "Data");
    assert_eq!(field(&meta, "route").fieldtype, "Data");
    assert_eq!(field(&meta, "is_default").fieldtype, "Check");
    assert_eq!(
        field(&meta, "style").fieldtype,
        "JSON",
        "the board style the feed projects is a JSON `DashboardStyle`"
    );
    let widgets = field(&meta, "widgets");
    assert_eq!(widgets.fieldtype, "Table");
    assert_eq!(
        widgets.options.as_deref(),
        Some(DASHBOARD_WIDGET),
        "the child table is the `Dashboard Widget` doctype"
    );

    let child = ferris_schema::doctypes::dashboard_widget_meta();
    assert_eq!(child.name, DASHBOARD_WIDGET);
    assert!(
        child.is_child_table,
        "a widget placement only lives in a parent"
    );
    assert_eq!(field(&child, "widget").fieldtype, "Data");
    assert!(
        field(&child, "widget").reqd,
        "a widget instance must reference a definition"
    );
    assert_eq!(field(&child, "params").fieldtype, "JSON");
    assert_eq!(
        field(&child, "style").fieldtype,
        "JSON",
        "the instance style the feed projects is a JSON `StyleSpec`"
    );
    assert_eq!(field(&child, "span").fieldtype, "Int");
}

// --- conversion (pure) -----------------------------------------------------

#[test]
fn a_parent_row_becomes_a_feed_dashboard() {
    let value = Value::from_json(serde_json::json!({
        "name": "sales",
        "label": "",
        "route": "/sales",
        "is_default": true,
        "style": { "background": "muted", "gap_px": 20 },
        "widgets": [
            { "idx": 2, "widget_id": "kpi", "widget": "host.stat",
              "label": "Open", "params": [["doctype", "ToDo"]], "span": 3, "col": 0, "row": 0, "rows": 4,
              "style": { "background": "accent", "padding": 8 } },
            { "idx": 3, "widget": "host.stat", "span": 6 },
        ],
    }));
    let dashboard = to_dashboard(&value).expect("a named row is a dashboard");

    assert_eq!(dashboard.id, "sales");
    assert_eq!(
        dashboard.label, "sales",
        "a blank label falls back to the id"
    );
    assert_eq!(dashboard.route, "/sales");
    assert!(dashboard.default);
    // The board style the document declares is projected, field by field.
    let board = dashboard.style.as_ref().expect("the declared board style");
    assert_eq!(board.background.as_deref(), Some("muted"));
    assert_eq!(board.gap_px, Some(20));
    assert_eq!(dashboard.widgets.len(), 2);

    let first = &dashboard.widgets[0];
    assert_eq!(first.id, "kpi");
    assert_eq!(first.widget, "host.stat");
    assert_eq!(first.label.as_deref(), Some("Open"));
    assert_eq!(
        first.params,
        vec![("doctype".to_string(), "ToDo".to_string())]
    );
    assert_eq!(
        (first.span, first.col, first.row, first.rows),
        (Some(3), Some(0), Some(0), Some(4))
    );
    // The instance style the row declares is projected too.
    let tile = first.style.as_ref().expect("the declared instance style");
    assert_eq!(tile.background.as_deref(), Some("accent"));
    assert_eq!(tile.padding, Some(8));

    let second = &dashboard.widgets[1];
    assert_eq!(
        second.id, "row-3",
        "a blank widget_id falls back to the row index"
    );
    assert!(second.col.is_none() && second.row.is_none());
    assert!(
        second.style.is_none(),
        "a row with no style keeps none and inherits its definition's"
    );
}

/// A style field holding the serialized string a hand-authored row may carry
/// reads the same as the object a `JSON` field stores, and a malformed one
/// degrades to "no declared style" instead of breaking the feed.
#[test]
fn style_fields_read_objects_strings_and_garbage() {
    let value = Value::from_json(serde_json::json!({
        "name": "sales",
        "route": "/sales",
        "style": "{\"gap_px\":12}",
        "widgets": [
            { "idx": 1, "widget": "host.stat", "style": "not json" },
        ],
    }));
    let dashboard = to_dashboard(&value).expect("a named row is a dashboard");
    assert_eq!(
        dashboard.style.as_ref().and_then(|style| style.gap_px),
        Some(12),
        "the serialized object parses"
    );
    assert!(
        dashboard.widgets[0].style.is_none(),
        "garbage reads as no style, not as a broken feed"
    );
}

#[test]
fn a_nameless_row_is_not_a_dashboard() {
    let value = Value::from_json(serde_json::json!({ "name": "  ", "route": "/x" }));
    assert!(to_dashboard(&value).is_none());
    assert!(to_dashboard(&Value::Null).is_none());
}

#[test]
fn widget_rows_without_a_reference_are_dropped_and_geometry_is_clamped() {
    let value = Value::from_json(serde_json::json!({
        "name": "board",
        "route": "/board",
        "widgets": [
            { "idx": 1, "widget": "" },
            { "idx": 2 },
            { "idx": 3, "widget": "host.stat", "span": 99, "col": 40, "rows": 0 },
            { "idx": 4, "widget": "host.stat", "span": -3 },
        ],
    }));
    let dashboard = to_dashboard(&value).expect("named");
    assert_eq!(
        dashboard.widgets.len(),
        2,
        "reference-less rows are dropped"
    );

    // `span` clamps into 1..=12, `col` into 0..=11, `rows` up to u8::MAX; a
    // negative span is not a `u8` and stays absent for the layout rule to fill.
    assert_eq!(dashboard.widgets[0].span, Some(12));
    assert_eq!(dashboard.widgets[0].col, Some(11));
    assert_eq!(dashboard.widgets[0].rows, Some(1));
    assert_eq!(dashboard.widgets[1].span, None);
}

#[test]
fn params_accept_pairs_and_objects_and_drop_non_scalars() {
    let value = Value::from_json(serde_json::json!({
        "name": "board",
        "route": "/board",
        "widgets": [{
            "idx": 1,
            "widget": "host.stat",
            "params": [
                ["doctype", "ToDo"],
                ["limit", 50],
                { "name": "flag", "value": true },
                ["nested", { "a": 1 }],
                ["", "ignored"],
                { "value": "no-name" },
            ],
        }],
    }));
    let dashboard = to_dashboard(&value).expect("named");
    assert_eq!(
        dashboard.widgets[0].params,
        vec![
            ("doctype".to_string(), "ToDo".to_string()),
            ("limit".to_string(), "50".to_string()),
            ("flag".to_string(), "true".to_string()),
            ("nested".to_string(), String::new()),
        ]
    );
}

#[test]
fn a_check_field_reads_bool_and_legacy_string_spellings() {
    for (raw, expected) in [
        (serde_json::json!(true), true),
        (serde_json::json!(false), false),
        (serde_json::json!("1"), true),
        (serde_json::json!("true"), true),
        (serde_json::json!("0"), false),
    ] {
        let value = Value::from_json(serde_json::json!({
            "name": "board", "route": "/board", "is_default": raw,
        }));
        assert_eq!(
            to_dashboard(&value).expect("named").default,
            expected,
            "is_default = {raw}"
        );
    }
}

// --- registry rules (pure) -------------------------------------------------

#[test]
fn registry_rules_drop_unusable_and_duplicate_routes() {
    let kept = apply_registry_rules(
        vec![
            dashboard("no-slash", "sales", false),
            dashboard("home", "/", false),
            dashboard("reserved", "/api/x", false),
            dashboard("taken", "/taken", false),
            dashboard("first", "/first", true),
            dashboard("second", "/second", true),
            dashboard("dup", "/first", false),
        ],
        HashSet::from(["/taken".to_string()]),
        false,
    );
    let ids: Vec<&str> = kept.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, vec!["first", "second"]);
    assert!(kept[0].default, "the first default keeps its flag");
    assert!(
        !kept[1].default,
        "a second default is cleared, not dropped: the dashboard still shows"
    );
}

#[test]
fn a_registry_default_clears_every_document_default() {
    let kept = apply_registry_rules(vec![dashboard("only", "/only", true)], HashSet::new(), true);
    assert_eq!(kept.len(), 1);
    assert!(
        !kept[0].default,
        "the registry's default wins; the document keeps its dashboard"
    );
}

// --- database --------------------------------------------------------------

#[tokio::test]
async fn document_dashboards_are_served_under_the_db_owner() {
    let db = pool().await;
    insert(
        &db,
        "board-b",
        "/doc-b",
        false,
        vec![widget_row("kpi", "host.stat", 3, 0, 0)],
    )
    .await;
    insert(
        &db,
        "board-a",
        "/doc-a",
        false,
        vec![widget_row("trend", "host.trend", 6, 0, 0)],
    )
    .await;

    let app = db_app(&db).await.expect("documents produce one app");
    assert_eq!(app.owner, OWNER);
    assert_eq!(app.owner, "db");
    assert_eq!(
        app.dashboards
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        vec!["board-a", "board-b"],
        "served in document-name order, so the revision is stable"
    );
    let first = &app.dashboards[0];
    assert_eq!(first.route, "/doc-a");
    assert_eq!(first.widgets.len(), 1);
    assert_eq!(first.widgets[0].id, "trend");
    assert_eq!(first.widgets[0].span, Some(6));

    // An install with no `Dashboard` rows adds nothing to the feed.
    let empty = pool().await;
    assert!(db_app(&empty).await.is_none());
}

#[tokio::test]
async fn dashboard_widgets_is_the_seed_for_the_layout_endpoint() {
    let pool = pool().await;
    insert(
        &pool,
        "board",
        "/doc-board",
        false,
        vec![
            widget_row("a", "host.stat", 4, 0, 0),
            widget_row("b", "host.trend", 8, 4, 0),
        ],
    )
    .await;

    let widgets = dashboard_widgets(&pool, "board")
        .await
        .expect("the document exists");
    assert_eq!(widgets.len(), 2);
    assert_eq!(widgets[0].id, "a");
    assert_eq!(widgets[1].widget, "host.trend");

    // A dashboard nobody declared has no seed — not an error.
    assert!(dashboard_widgets(&pool, "missing").await.is_none());
}
