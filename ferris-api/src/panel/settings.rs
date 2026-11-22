//! `GET|PUT /api/notification-settings` — the per-user notification flags.

use axum::{
    Json,
    extract::{FromRef, State},
    response::{IntoResponse, Response},
};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::Value;
use ferris_document::DocumentEngine;

use super::context::{caller_context, panel_context, settings_view};
use crate::ApiState;
use crate::core_doctypes::NOTIFICATION_SETTINGS;
use crate::store::{document_exists, effective_meta};

/// `GET /api/notification-settings` — the signed-in user's settings, or the
/// defaults when they have never saved any.
pub async fn get_notification_settings<S>(
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
    let meta = effective_meta(&state.pool, NOTIFICATION_SETTINGS)
        .await
        .ok()
        .flatten();
    let read_ctx = caller_context(NOTIFICATION_SETTINGS, meta.as_ref(), &user.0);
    let stored = engine.get(&read_ctx, &user_id).await.ok();
    Json(settings_view(stored.as_ref(), &user_id)).into_response()
}

/// `PUT /api/notification-settings` — upsert the signed-in user's settings.
pub async fn put_notification_settings<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    user: AuthenticatedUser,
    Json(body): Json<Value>,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    let user_id = user.0.id.clone();
    let engine = DocumentEngine::new(state.pool.clone());
    let meta = effective_meta(&state.pool, NOTIFICATION_SETTINGS)
        .await
        .ok()
        .flatten();
    let ctx = panel_context(NOTIFICATION_SETTINGS, meta.as_ref());

    let flag = |key: &str| body.get(key).and_then(|v| v.as_bool()).unwrap_or(true);
    let mut data = Value::from_json(serde_json::json!({
        "user": user_id,
        "desktop": flag("desktop"),
        "notify_on_login": flag("notify_on_login"),
    }));

    let read_ctx = caller_context(NOTIFICATION_SETTINGS, meta.as_ref(), &user.0);
    let exists = match document_exists(&engine, &read_ctx, &user_id).await {
        Ok(exists) => exists,
        Err(e) => return crate::router::error_response(e),
    };
    let result = if exists {
        engine.update(&ctx, &user_id, data).await
    } else {
        data.set("name".to_string(), Value::from(user_id.as_str()));
        engine.create_owned_by(&ctx, &user_id, data).await
    };

    match result {
        Ok(saved) => Json(settings_view(Some(&saved), &user_id)).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}
