//! The per-user desk grid: `GET|PUT /api/desk-layout`.

use axum::{
    Json,
    extract::{FromRef, State},
    response::{IntoResponse, Response},
};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::Value;
use ferris_db::Db;
use ferris_document::DocumentEngine;

use ferris_schema::contract::{DeskIconPlacement, DeskLayout};

use super::context::{panel_context, subject_context};
use crate::ApiState;
use crate::core_doctypes::DESK_LAYOUT;
use crate::store::{document_exists, effective_meta};

/// `GET /api/desk-layout` — the signed-in user's desk: the grid they arranged,
/// over the workspaces the loaded apps declared.
///
/// A user who has never moved an icon gets the seed exactly as an app declared
/// it, so the grid is useful before anyone has edited it.
pub async fn get_desk_layout<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    user: AuthenticatedUser,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    let user_id = user.0.id.clone();
    let saved = stored_layout(&state.pool, &user_id).await;
    Json(crate::desk::load_layout(&state.pool, saved.as_deref()).await).into_response()
}

/// `PUT /api/desk-layout` — save the signed-in user's arrangement of the grid.
///
/// The body is `{"icons": [ ...placements... ]}`, where each placement names
/// either a workspace or a folder. Both paths below derive the row from the
/// resolved user id, so a request body cannot widen the scope to someone else's
/// desk — the same reason the notification settings resolve their target from
/// the session rather than the payload.
pub async fn put_desk_layout<S>(
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
    let icons = body.get("icons").cloned().unwrap_or(Value::Null);
    let saved: Vec<DeskIconPlacement> = serde_json::from_value(icons.to_json()).unwrap_or_default();

    match save_layout(&state.pool, &user_id, &saved).await {
        Ok(layout) => Json(layout).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// Store `user_id`'s arrangement and answer with the grid it produced.
///
/// The effective grid comes back rather than a bare acknowledgement because a
/// save can change it beyond what was asked: placements naming workspaces this
/// install no longer has are dropped, and workspaces the arrangement never
/// mentioned are appended. Sending that back is what stops the client from
/// drawing a grid the server will not reproduce on the next load.
pub(super) async fn save_layout(
    pool: &Db,
    user_id: &str,
    saved: &[DeskIconPlacement],
) -> ferris_core::Result<DeskLayout> {
    let effective = crate::desk::load_layout(pool, Some(saved)).await;
    let layout_json =
        serde_json::to_string(&placements_of(&effective)).unwrap_or_else(|_| "[]".to_string());

    let engine = DocumentEngine::new(pool.clone());
    let meta = effective_meta(pool, DESK_LAYOUT).await.ok().flatten();
    let ctx = panel_context(DESK_LAYOUT, meta.as_ref());

    let mut data = Value::from_json(serde_json::json!({
        "user": user_id,
        "layout": layout_json,
    }));

    let read_ctx = subject_context(DESK_LAYOUT, meta.as_ref(), user_id);
    if document_exists(&engine, &read_ctx, user_id).await? {
        engine.update(&ctx, user_id, data).await?;
    } else {
        data.set("name".to_string(), Value::from(user_id));
        engine.create_owned_by(&ctx, user_id, data).await?;
    }
    Ok(effective)
}

/// Read the stored placements back out of a layout document.
///
/// The field holds JSON as a string, so a hand-edited or truncated value has to
/// parse to "no arrangement" rather than to an error: the seed is a perfectly
/// good desk, and refusing to render one over a bad string would be worse than
/// showing the app's default.
fn saved_placements(doc: &Value) -> Option<Vec<DeskIconPlacement>> {
    let raw = doc.get("layout")?.as_str()?.to_string();
    serde_json::from_str(&raw).ok()
}

/// One user's saved grid, or `None` when they have never saved one.
pub async fn stored_layout(pool: &Db, user_id: &str) -> Option<Vec<DeskIconPlacement>> {
    let engine = DocumentEngine::new(pool.clone());
    let meta = effective_meta(pool, DESK_LAYOUT).await.ok().flatten();
    let doc = engine
        .get(
            &subject_context(DESK_LAYOUT, meta.as_ref(), user_id),
            user_id,
        )
        .await
        .ok()?;
    saved_placements(&doc)
}

/// Fold a rendered grid back into the placements the client saves, so what is
/// stored is exactly what was drawn: the unmentioned workspaces the merge
/// appended are now explicit, and members that no longer exist are gone.
fn placements_of(layout: &DeskLayout) -> Vec<DeskIconPlacement> {
    layout
        .icons
        .iter()
        .map(|icon| {
            if icon.is_folder() {
                DeskIconPlacement {
                    folder: icon.id.clone(),
                    label: icon.label.clone(),
                    icon: icon.icon.clone().unwrap_or_default(),
                    color: icon.color.clone().unwrap_or_default(),
                    items: icon.items.iter().map(|i| i.id.clone()).collect(),
                    ..Default::default()
                }
            } else {
                DeskIconPlacement {
                    workspace: icon.id.clone(),
                    hidden: icon.hidden,
                    ..Default::default()
                }
            }
        })
        .collect()
}
