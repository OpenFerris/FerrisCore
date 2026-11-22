//! The initial widget migration against a real engine and database.
//!
//! The unit tests in `ferris-api/src/widgets/tests.rs` cover the pure
//! row/spec/contract conversions. This test exercises the part that only a
//! database can prove: `sync` creates the `Widget` row through the document
//! engine, `list_apps` reads it back, and a second `sync` leaves an
//! administrator's edit alone because the seed hash did not change.

// Integration tests are their own crate; most of the library's dependencies
// are not referenced by this one test module.
#![allow(unused_crate_dependencies)]

use ferris_core::Value;
use ferris_core::ui_extensions::{StyleSpec, WidgetDefSpec, WidgetParamSpec, replace_widgets};

fn def() -> WidgetDefSpec {
    WidgetDefSpec {
        name: "sales-chart".into(),
        label: "Ventas".into(),
        component: "sales-chart".into(),
        kind: "chart".into(),
        events: vec!["refresh".into()],
        style: Some(StyleSpec {
            background: Some("card".into()),
            padding: Some(12),
            ..StyleSpec::default()
        }),
        params: vec![WidgetParamSpec {
            name: "doctype".into(),
            label: Some("DocType".into()),
            value_type: "doctype".into(),
            required: true,
            ..WidgetParamSpec::default()
        }],
        code_path: Some("ui/widgets/charts/sales.js".into()),
        script: Some("export const x = 1;".into()),
        version: Some("1".into()),
        ..WidgetDefSpec::default()
    }
}

/// A first `sync` seeds the row; a Desk edit then survives a second `sync`
/// because only the seed-controlled fields are compared by `seed_hash`.
#[tokio::test]
async fn seeding_upserts_and_an_admin_edit_survives_a_second_sync() {
    let pool = ferris_db::memory_db().await.expect("in-memory database");
    ferris_schema::register_doctype(ferris_schema::doctypes::widget_meta());
    ferris_schema::register_doctype(ferris_schema::doctypes::widget_param_meta());
    assert!(
        ferris_schema::get_doctype(ferris_api::widgets::WIDGET_PARAM).is_some(),
        "the Widget Param child table is part of the schema"
    );
    let owner = "widget-integration-test";
    let name = "widget-integration-test.sales-chart";
    replace_widgets(owner, vec![def()]);

    assert!(
        ferris_api::widgets::sync(&pool).await,
        "the seed migration completes"
    );
    let apps = ferris_api::widgets::list_apps(&pool)
        .await
        .expect("the Widget rows are readable");
    let app = apps
        .iter()
        .find(|app| app.owner == owner)
        .expect("the seeded app is in the feed");
    assert_eq!(app.widgets.len(), 1);
    assert_eq!(app.widgets[0].label, "Ventas");
    assert_eq!(app.widgets[0].params.len(), 1);
    assert_eq!(app.widgets[0].params[0].name, "doctype");
    assert_eq!(app.widgets[0].params[0].value_type, "doctype");
    assert_eq!(app.widgets[0].events, vec!["refresh".to_string()]);
    assert_eq!(
        app.widgets[0]
            .style
            .as_ref()
            .and_then(|style| style.background.as_deref()),
        Some("card")
    );
    let code_url = app.widgets[0]
        .code_url
        .as_deref()
        .expect("the seeded widget has code");
    assert!(
        code_url
            .starts_with("/plugins/widget-integration-test/asset/ui/widgets/charts/sales.js?v="),
        "the source is served at its original path with a content version: {code_url}"
    );

    // Simulate a Desk edit of a presentation field. The seed hash stays the
    // same, so the next `sync` has nothing to update.
    let meta = ferris_schema::get_doctype("Widget");
    let mut ctx = ferris_document::EngineContext::new("Widget", "");
    ctx.meta = meta.as_deref();
    let mut admin = ferris_auth::User::system_user("admin@local".to_string(), "Admin".to_string());
    admin.roles = vec!["System Manager".to_string()];
    ctx.user = Some(admin);
    ferris_document::DocumentEngine::new(pool.clone())
        .update(&ctx, name, ferris_core::value!({ "label": "Editado" }))
        .await
        .expect("the admin edit is written");

    assert!(
        ferris_api::widgets::sync(&pool).await,
        "the seed migration completes"
    );
    let apps = ferris_api::widgets::list_apps(&pool)
        .await
        .expect("the Widget rows are readable");
    let app = apps
        .iter()
        .find(|app| app.owner == owner)
        .expect("the seeded app is in the feed");
    assert_eq!(
        app.widgets[0].label, "Editado",
        "a sync with an unchanged seed must not overwrite an admin edit"
    );

    // A changed seed is a different hash, so the next sync updates the row:
    // the stored source follows the app's new definition.
    let mut changed = def();
    changed.script = Some("export const x = 2;".into());
    replace_widgets(owner, vec![changed]);
    assert!(
        ferris_api::widgets::sync(&pool).await,
        "the changed seed updates the row"
    );
    let row = ferris_document::DocumentEngine::new(pool.clone())
        .get(&ctx, name)
        .await
        .expect("the updated row is readable");
    assert_eq!(
        row.get("script").and_then(Value::as_str),
        Some("export const x = 2;")
    );
}
