//! Notification feed, mark-read endpoints and the login-time producer.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{FromRef, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::Value;
use ferris_db::Db;
use ferris_document::{DocumentEngine, ListQuery};

use ferris_schema::contract::{Notification, NotificationFeed};

use super::context::{
    FEED_LIMIT, caller_context, panel_context, str_field, subject_context, to_notif,
};
use crate::ApiState;
use crate::core_doctypes::{NOTIFICATION, NOTIFICATION_SETTINGS};
use crate::store::effective_meta;

/// Whether the user wants the login notification. A missing settings document
/// means enabled, matching the field default.
pub async fn notify_on_login(pool: &Db, user_id: &str) -> bool {
    let engine = DocumentEngine::new(pool.clone());
    let meta = effective_meta(pool, NOTIFICATION_SETTINGS)
        .await
        .ok()
        .flatten();
    engine
        .get(
            &subject_context(NOTIFICATION_SETTINGS, meta.as_ref(), user_id),
            user_id,
        )
        .await
        .ok()
        .and_then(|d| d.get("notify_on_login").and_then(|v| v.as_bool()))
        .unwrap_or(true)
}

/// Create a notification addressed to `user_id`, owned by them.
///
/// Authored as the service user because the producer runs outside any
/// authenticated request (at login). The document is owned by the *recipient*,
/// not by the service user, so that the owner rule in
/// `ferris_auth::permissions::check_permission` lets them mark it read later.
/// That is why this calls `create_owned_by` rather than `create`: a
/// `Notification` is created *for* its subject, not *by* it.
///
/// The recipient is passed as an argument rather than smuggled in as an
/// `owner` key in the body — the engine rejects a body that disagrees with the
/// owner it was given (§C1).
pub async fn push_notification(
    pool: &Db,
    user_id: &str,
    title: &str,
    description: &str,
    route: &str,
) -> ferris_core::Result<()> {
    let meta = effective_meta(pool, NOTIFICATION).await?;
    let engine = DocumentEngine::new(pool.clone());
    let data = Value::from_json(serde_json::json!({
        "user": user_id,
        "notification_type": "General",
        "title": title,
        "description": description,
        "route": route,
        "is_read": false,
    }));
    let created = engine
        .create_owned_by(&panel_context(NOTIFICATION, meta.as_ref()), user_id, data)
        .await?;
    // Hand the stored row to the live transport, if one is installed, so an open
    // Desk session sees it without waiting for a refetch. The publish is a
    // no-op when no server has wired the bus (tests, embedded engine).
    let payload = serde_json::to_value(to_notif(&created)).unwrap_or(serde_json::Value::Null);
    crate::notifications::publish(user_id, Value::from_json(payload));
    Ok(())
}

/// `GET /api/notifications` — the signed-in user's feed plus the unread count.
pub async fn notifications<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    user: AuthenticatedUser,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    let user_id = user.0.id.clone();
    let engine = DocumentEngine::new(state.pool.clone());

    let feed_query = ListQuery {
        filters: HashMap::from([("user".to_string(), Value::from(user_id.as_str()))]),
        order_by: Some("creation desc".to_string()),
        limit: Some(FEED_LIMIT),
        ..Default::default()
    };
    let meta = effective_meta(&state.pool, NOTIFICATION)
        .await
        .ok()
        .flatten();
    let read_ctx = caller_context(NOTIFICATION, meta.as_ref(), &user.0);
    let items: Vec<Notification> = engine
        .list(&read_ctx, &feed_query)
        .await
        .map(|(docs, _)| docs.iter().map(to_notif).collect())
        .unwrap_or_default();

    // The unread badge counts every unread notification, not just the loaded
    // page, so it is a separate count over the same user filter.
    let unread_query = ListQuery {
        filters: HashMap::from([
            ("user".to_string(), Value::from(user_id.as_str())),
            ("is_read".to_string(), Value::from(false)),
        ]),
        limit: Some(1),
        ..Default::default()
    };
    let unread = engine
        .list(&read_ctx, &unread_query)
        .await
        .map(|(_, total)| total)
        .unwrap_or(0);

    Json(NotificationFeed { items, unread }).into_response()
}

/// `POST /api/notifications/read` — mark every notification of the signed-in
/// user as read.
pub async fn mark_notifications_read<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    user: AuthenticatedUser,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    let user_id = user.0.id.clone();
    let engine = DocumentEngine::new(state.pool.clone());
    let meta = effective_meta(&state.pool, NOTIFICATION)
        .await
        .ok()
        .flatten();
    let ctx = panel_context(NOTIFICATION, meta.as_ref());

    let unread_query = ListQuery {
        filters: HashMap::from([
            ("user".to_string(), Value::from(user_id.as_str())),
            ("is_read".to_string(), Value::from(false)),
        ]),
        ..Default::default()
    };
    let read_ctx = caller_context(NOTIFICATION, meta.as_ref(), &user.0);
    let docs = engine
        .list(&read_ctx, &unread_query)
        .await
        .map(|(docs, _)| docs)
        .unwrap_or_default();

    for doc in docs {
        if let Some(name) = doc.get("name").and_then(|v| v.as_str()) {
            let data = Value::from_json(serde_json::json!({ "is_read": true }));
            // Best effort per row: one failure must not strand the rest.
            if let Err(e) = engine.update(&ctx, name, data).await {
                tracing::warn!(error = %e, name = name, "failed to mark notification read");
            }
        }
    }

    StatusCode::NO_CONTENT.into_response()
}

/// Whether `name` is a notification belonging to `user`.
///
/// The read runs as the caller so the engine's owner rule applies, and the
/// `user` field is checked explicitly as well: a `Notification` is owned by its
/// *recipient*, so the owner match is what stops one user from touching
/// another's row by name.
async fn owns_notification(pool: &Db, user: &ferris_auth::User, name: &str) -> bool {
    let engine = DocumentEngine::new(pool.clone());
    let meta = effective_meta(pool, NOTIFICATION).await.ok().flatten();
    let read_ctx = caller_context(NOTIFICATION, meta.as_ref(), user);
    engine
        .get(&read_ctx, name)
        .await
        .is_ok_and(|doc| str_field(&doc, "user") == user.id)
}

/// Update a **single** notification of the signed-in user.
async fn update_owned_notification(
    pool: &Db,
    user: &ferris_auth::User,
    name: &str,
    data: Value,
) -> Response {
    if !owns_notification(pool, user, name).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    let engine = DocumentEngine::new(pool.clone());
    let meta = effective_meta(pool, NOTIFICATION).await.ok().flatten();
    let ctx = panel_context(NOTIFICATION, meta.as_ref());
    match engine.update(&ctx, name, data).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `POST /api/notifications/{name}/read` — mark one notification as read.
pub async fn mark_notification_read<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    Path(name): Path<String>,
    user: AuthenticatedUser,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    update_owned_notification(
        &state.pool,
        &user.0,
        &name,
        Value::from_json(serde_json::json!({ "is_read": true })),
    )
    .await
}

/// `POST /api/notifications/{name}/dismiss` — remove one of the signed-in user's
/// notifications from their feed.
pub async fn dismiss_notification<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    Path(name): Path<String>,
    user: AuthenticatedUser,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if !owns_notification(&state.pool, &user.0, &name).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    let engine = DocumentEngine::new(state.pool.clone());
    let meta = effective_meta(&state.pool, NOTIFICATION)
        .await
        .ok()
        .flatten();
    let ctx = panel_context(NOTIFICATION, meta.as_ref());
    match engine.delete(&ctx, &name).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => crate::router::error_response(e),
    }
}
