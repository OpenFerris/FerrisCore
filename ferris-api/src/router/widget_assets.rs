use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::ApiState;

/// `GET /api/widgets/{app}/{name}/script.js` — the stored module source.
///
/// Served only when the `Widget` row exists, is enabled and carries a script.
/// A widget that declares a `code_path` is loaded from the plugin asset route
/// instead, so relative `import`s keep resolving; this endpoint is the
/// self-contained fallback.
pub(super) async fn widget_script(
    State(state): State<Arc<ApiState>>,
    Path((app, name)): Path<(String, String)>,
) -> Response {
    crate::widgets::script_by_name(&state.pool, &app, &name)
        .await
        .map_or_else(
            || (StatusCode::NOT_FOUND, "widget script not found").into_response(),
            |script| widget_asset("application/javascript; charset=utf-8", script),
        )
}

/// `GET /api/widgets/{app}/{name}/style.css` — the stored widget CSS.
pub(super) async fn widget_style(
    State(state): State<Arc<ApiState>>,
    Path((app, name)): Path<(String, String)>,
) -> Response {
    crate::widgets::stylesheet_by_name(&state.pool, &app, &name)
        .await
        .map_or_else(
            || (StatusCode::NOT_FOUND, "widget stylesheet not found").into_response(),
            |css| widget_asset("text/css; charset=utf-8", css),
        )
}

/// A widget source response: `no-store`, because the row is editable and a
/// stale module would keep an old custom element alive.
fn widget_asset(content_type: &'static str, body: String) -> Response {
    (
        [
            (axum::http::header::CONTENT_TYPE, content_type),
            (axum::http::header::CACHE_CONTROL, "no-store, max-age=0"),
        ],
        body,
    )
        .into_response()
}
