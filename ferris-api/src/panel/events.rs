//! `GET /api/events` — the upcoming-events feed.

use axum::{
    Json,
    extract::{FromRef, State},
    response::{IntoResponse, Response},
};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::Value;
use ferris_document::{DocumentEngine, FilterOp, ListFilter, ListQuery};

use ferris_schema::contract::{DeskEvent, EventFeed};

use super::context::{EVENT_LIMIT, caller_context, str_field, to_event};
use crate::ApiState;
use crate::core_doctypes::EVENT;
use crate::store::effective_meta;

/// `GET /api/events` — upcoming events: shared ones plus the user's own.
pub async fn events<S>(
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
    let now = chrono::Utc::now().to_rfc3339();
    let meta = effective_meta(&state.pool, EVENT).await.ok().flatten();
    let read_ctx = caller_context(EVENT, meta.as_ref(), &user.0);

    let query = ListQuery {
        filter_ops: vec![ListFilter {
            field: "starts_on".to_string(),
            op: FilterOp::Gte,
            value: Value::from(now),
            or: false,
        }],
        order_by: Some("starts_on asc".to_string()),
        limit: Some(EVENT_LIMIT),
        ..Default::default()
    };

    // An event with no `user` is shared; one with a `user` is that person's.
    // The `user` filter below narrows what the caller is shown; the engine's
    // `Permission::Read` check decides what the caller is *allowed* to see.
    let items: Vec<DeskEvent> = engine
        .list(&read_ctx, &query)
        .await
        .map(|(docs, _)| {
            docs.into_iter()
                .filter(|d| {
                    let owner = str_field(d, "user");
                    owner.is_empty() || owner == user_id
                })
                .map(|d| to_event(&d))
                .collect()
        })
        .unwrap_or_default();

    Json(EventFeed { items }).into_response()
}
