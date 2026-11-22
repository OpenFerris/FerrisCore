//! Reading the saved row of one user's dashboard layout.
//!
//! The row is system-owned (the user is part of its name, see
//! [`super::layout::layout_name`]), so it is read through the service principal
//! — the same context the write uses — and never through the caller. The
//! per-user key is what isolates two people's arrangements of the same
//! dashboard; the service principal is what lets the second save update rather
//! than collide.

use ferris_db::Db;
use ferris_document::DocumentEngine;
use ferris_schema::contract::{DashboardConfig, WidgetInstance};

use crate::core_doctypes::DASHBOARD_LAYOUT;
use crate::panel::panel_context;
use crate::store::effective_meta;

use super::layout::layout_name;

/// The saved row for one user's layout of one dashboard, or `None` when that
/// user has not saved one.
///
/// The widgets and the board override are held together because they are the
/// same state of the same board.
pub(super) struct StoredRow {
    pub(super) widgets: Vec<WidgetInstance>,
    pub(super) config: DashboardConfig,
}

pub(super) async fn stored_row(
    pool: &Db,
    owner: &str,
    dashboard: &str,
    user: &str,
) -> Option<StoredRow> {
    let engine = DocumentEngine::new(pool.clone());
    let meta = effective_meta(pool, DASHBOARD_LAYOUT).await.ok().flatten();
    let name = layout_name(owner, dashboard, user);
    let doc = engine
        .get(&panel_context(DASHBOARD_LAYOUT, meta.as_ref()), &name)
        .await
        .ok()?;
    // A missing or malformed `widgets` string reads as an empty list, matching
    // the Desk's "a bad row is a seed-level board, not an error" stance; the
    // config is optional and absent means "no override".
    let widgets = doc
        .get("widgets")
        .and_then(|raw| raw.as_str())
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    let config = doc
        .get("config")
        .and_then(|raw| raw.as_str())
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    Some(StoredRow { widgets, config })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_core::Value;
    use ferris_document::DocumentEngine;

    /// Write one user's saved layout of one dashboard, as the PUT handler does.
    async fn save(pool: &Db, owner: &str, dashboard: &str, user: &str, widget: &str) {
        let engine = DocumentEngine::new(pool.clone());
        let meta = effective_meta(pool, DASHBOARD_LAYOUT).await.ok().flatten();
        let ctx = panel_context(DASHBOARD_LAYOUT, meta.as_ref());
        let widgets = serde_json::json!([{ "id": widget, "widget": "host.stat" }]).to_string();
        engine
            .create(
                &ctx,
                Value::from_json(serde_json::json!({
                    "name": layout_name(owner, dashboard, user),
                    "app_owner": owner,
                    "dashboard": dashboard,
                    "widgets": widgets,
                    "config": "{}",
                })),
            )
            .await
            .unwrap();
    }

    /// API-10: the saved layout is per user. Two users saving the same app
    /// dashboard write two rows, and reading one never answers with the other's
    /// arrangement.
    #[tokio::test]
    async fn two_users_keep_distinct_layouts_of_the_same_dashboard() {
        let pool = ferris_db::memory_db().await.unwrap();
        ferris_schema::register_doctype(crate::doctypes::dashboard_layout_meta());

        save(&pool, "app", "board", "ada@example.com", "ada-chart").await;
        save(&pool, "app", "board", "grace@example.com", "grace-table").await;

        let ada = stored_row(&pool, "app", "board", "ada@example.com")
            .await
            .expect("ada has a saved layout");
        let grace = stored_row(&pool, "app", "board", "grace@example.com")
            .await
            .expect("grace has a saved layout");
        assert_eq!(ada.widgets.len(), 1);
        assert_eq!(ada.widgets[0].id, "ada-chart");
        assert_eq!(grace.widgets.len(), 1);
        assert_eq!(grace.widgets[0].id, "grace-table");
        assert_ne!(ada.widgets[0].id, grace.widgets[0].id);

        // A user who never saved falls back to the seed, not to a neighbour's
        // row.
        assert!(
            stored_row(&pool, "app", "board", "linus@example.com")
                .await
                .is_none()
        );
    }
}
