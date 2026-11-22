//! Panel tests: notification scoping, settings defaults and the saved desk grid.

use std::collections::HashMap;

use ferris_core::Value;
use ferris_core::workspaces::WorkspaceSpec;
use ferris_db::Db;
use ferris_document::{DocumentEngine, ListQuery, with_user};
use ferris_schema::contract::DeskIconPlacement;

use super::context::{SERVICE_ID, panel_context, str_field, subject_context};
use super::layout::{save_layout, stored_layout};
use super::{notify_on_login, push_notification};
use crate::core_doctypes::{DESK_LAYOUT, NOTIFICATION, NOTIFICATION_SETTINGS};

async fn pool() -> Db {
    ferris_db::memory_db().await.unwrap()
}

async fn admin() -> ferris_auth::User {
    let user = crate::service::service_user(SERVICE_ID, "System");
    with_user(Some(user.clone()), async {}).await;
    user
}

#[tokio::test]
async fn push_notification_creates_a_notification_the_feed_can_find() {
    crate::core_doctypes::register_core_doctypes();
    let pool = pool().await;
    let _ = admin().await;
    push_notification(&pool, "ada@example.com", "Hello", "Desc", "/desk")
        .await
        .unwrap();

    let engine = DocumentEngine::new(pool.clone());
    let query = ListQuery {
        filters: HashMap::from([("user".to_string(), Value::from("ada@example.com"))]),
        ..Default::default()
    };
    let (docs, total) = engine
        .list(
            &subject_context(NOTIFICATION, None, "ada@example.com"),
            &query,
        )
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(str_field(&docs[0], "title"), "Hello");
    assert_eq!(
        docs[0].get("is_read").and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[tokio::test]
async fn feed_is_scoped_to_the_recipient() {
    crate::core_doctypes::register_core_doctypes();
    let pool = pool().await;
    let _ = admin().await;
    push_notification(&pool, "ada@example.com", "For Ada", "", "/desk")
        .await
        .unwrap();
    push_notification(&pool, "grace@example.com", "For Grace", "", "/desk")
        .await
        .unwrap();

    let engine = DocumentEngine::new(pool.clone());
    let query = ListQuery {
        filters: HashMap::from([("user".to_string(), Value::from("ada@example.com"))]),
        ..Default::default()
    };
    let (docs, total) = engine
        .list(
            &subject_context(NOTIFICATION, None, "ada@example.com"),
            &query,
        )
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(str_field(&docs[0], "title"), "For Ada");
}

#[tokio::test]
async fn settings_round_trip_and_default_to_enabled() {
    crate::core_doctypes::register_core_doctypes();
    let pool = pool().await;
    let _ = admin().await;

    // Never saved: defaults on.
    assert!(notify_on_login(&pool, "ada@example.com").await);

    let engine = DocumentEngine::new(pool.clone());
    let ctx = panel_context(NOTIFICATION_SETTINGS, None);
    let data = Value::from_json(serde_json::json!({
        "name": "ada@example.com",
        "user": "ada@example.com",
        "desktop": true,
        "notify_on_login": false,
    }));
    // Owned by Ada even though the session is the service user — this is the
    // per-user settings row, so `create_owned_by` is the correct path.
    engine
        .create_owned_by(&ctx, "ada@example.com", data)
        .await
        .unwrap();

    assert!(!notify_on_login(&pool, "ada@example.com").await);
}

/// The arrangement a user saves is the arrangement they get back on the next
/// load: the grid the save answered with has to be reproducible from what
/// was stored, or the desk silently reshuffles itself on every reload.
#[tokio::test]
async fn a_saved_arrangement_comes_back_on_the_next_load() {
    crate::core_doctypes::register_core_doctypes();
    let pool = pool().await;
    let _ = admin().await;
    ferris_core::workspaces::register_app_workspaces(
        "layout-test",
        vec![ferris_core::workspaces::WorkspaceFolderSpec {
            name: "layout-seed".into(),
            ..Default::default()
        }],
        vec![
            WorkspaceSpec {
                name: "layout-sales".into(),
                module: "layout-test".into(),
                folder: Some("layout-seed".into()),
                ..Default::default()
            },
            WorkspaceSpec {
                name: "layout-stock".into(),
                module: "layout-test".into(),
                ..Default::default()
            },
        ],
    );

    let mine = DeskIconPlacement {
        folder: "mine".into(),
        label: "My things".into(),
        items: vec!["layout-sales".into()],
        ..Default::default()
    };
    let answer = save_layout(&pool, "ada@example.com", &[mine])
        .await
        .unwrap();
    let drawn: Vec<String> = answer
        .icons
        .iter()
        .filter(|i| i.id.starts_with("layout-") || i.id == "mine")
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(drawn, vec!["mine", "layout-stock"]);

    let stored = stored_layout(&pool, "ada@example.com")
        .await
        .expect("saved");
    let reloaded = crate::desk::build_layout(Some(&stored));
    let again: Vec<String> = reloaded
        .icons
        .iter()
        .filter(|i| i.id.starts_with("layout-") || i.id == "mine")
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(again, drawn, "the grid survives a reload unchanged");

    let mine = stored
        .iter()
        .find(|p| p.folder == "mine")
        .expect("the user's folder is stored");
    assert_eq!(mine.items, vec!["layout-sales".to_string()]);
    assert_eq!(
        mine.icon, "Folder",
        "what is stored is what was drawn, with the defaults filled in"
    );
}

/// Two people's desks are their own: one user's save cannot move or hide the
/// other's icons.
#[tokio::test]
async fn one_users_desk_is_not_anothers() {
    crate::core_doctypes::register_core_doctypes();
    let pool = pool().await;
    let _ = admin().await;
    ferris_core::workspaces::register_app_workspaces(
        "scope-test",
        vec![],
        vec![WorkspaceSpec {
            name: "scope-ws".into(),
            module: "scope-test".into(),
            ..Default::default()
        }],
    );

    let hidden = DeskIconPlacement {
        workspace: "scope-ws".into(),
        hidden: true,
        ..Default::default()
    };
    save_layout(&pool, "ada@example.com", &[hidden])
        .await
        .unwrap();

    assert!(stored_layout(&pool, "ada@example.com").await.is_some());
    assert!(
        stored_layout(&pool, "grace@example.com").await.is_none(),
        "Grace has never arranged anything"
    );
    let grace = crate::desk::build_layout(None);
    assert!(
        grace.icons.iter().any(|i| i.id == "scope-ws"),
        "Ada hiding a workspace does not hide it from Grace"
    );
}

/// A layout field that is not parseable JSON must not take the desk down: the
/// apps' seed is a working desk, and it is what the user gets back.
#[tokio::test]
async fn an_unreadable_layout_falls_back_to_the_seed() {
    crate::core_doctypes::register_core_doctypes();
    let pool = pool().await;
    let _ = admin().await;

    let engine = DocumentEngine::new(pool.clone());
    let ctx = panel_context(DESK_LAYOUT, None);
    engine
        .create_owned_by(
            &ctx,
            "ada@example.com",
            Value::from_json(serde_json::json!({
                "name": "ada@example.com",
                "user": "ada@example.com",
                "layout": "{not json",
            })),
        )
        .await
        .unwrap();

    assert!(
        stored_layout(&pool, "ada@example.com").await.is_none(),
        "a truncated layout reads as no arrangement, not as an error"
    );
    let fallback = crate::desk::build_layout(None);
    assert!(
        fallback
            .icons
            .iter()
            .all(|i| i.kind == "workspace" || i.kind == "folder"),
        "and the desk still draws from the apps' seed"
    );
}
