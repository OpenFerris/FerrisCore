//! Persistent per-entity desired state for pull-based edge agents (R-FW-1b).
//!
//! A control plane publishes the configuration one entity should converge on;
//! an edge agent polls for it and applies it. The framework provides the
//! durable, per-entity store and the pull surface, and knows nothing about what
//! an "entity" is or what its payload means — that is the deployment's contract.
//!
//! # Shape
//!
//! One document of the hidden [`DESIRED_STATE`](crate::doctypes::DESIRED_STATE)
//! DocType per entity, **named after the entity id**, so a pull is a primary-key
//! read. `payload` is the caller's JSON document (stored as text, so object,
//! array, string, number and bool all round-trip); `version` is a generation
//! counter bumped on every publish.
//!
//! # Pull protocol
//!
//! `GET /api/desired-state/{entity}` answers the current state with an `ETag`
//! of the quoted version. An agent that already holds a version sends it back
//! as `If-None-Match` and gets `304 Not Modified` when nothing changed, so a
//! tight poll costs one conditional request.
//!
//! `PUT /api/desired-state/{entity}` publishes (`System Manager` only), and an
//! optional `expected_version` turns it into a compare-and-set: a stale writer
//! gets `409 Conflict` instead of clobbering a newer generation.
//! `DELETE /api/desired-state/{entity}` clears an entity idempotently.
//!
//! # Permissions
//!
//! The DocType grants `All` users **read** — a pull is the whole point and any
//! signed-in agent may perform it — while writes stay with `System Manager`.
//! Because the store goes through the document engine, both halves obey the
//! live policy, and an operator can narrow the read again through the policy
//! document if a deployment considers desired state sensitive.
#![allow(clippy::doc_markdown)]

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use ferris_core::{DocumentError, Error, Result, Value};
use ferris_db::Db;
use serde::Deserialize;

use crate::doctypes::DESIRED_STATE;
use crate::router::{ApiState, error_response};
use crate::store::{DocStore, validate_name};
use crate::types::ListParams;

/// How many entities the control-plane list returns in one page.
const LIST_LIMIT: u32 = 1000;

/// One entity's desired state as it is published and pulled.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DesiredState {
    /// The entity id; also the document name.
    pub entity: String,
    /// Monotonic generation, bumped on every publish.
    pub version: u64,
    /// The desired configuration document.
    pub payload: Value,
    /// Last publish time (the engine's `modified` stamp).
    pub updated_at: Option<String>,
}

impl DesiredState {
    /// Decode a stored `Desired State` document.
    fn from_doc(doc: &Value) -> Result<Self> {
        let entity = doc
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let payload = match doc.get("payload").and_then(Value::as_str) {
            Some(text) if !text.is_empty() => serde_json::from_str(text)?,
            _ => Value::Null,
        };
        Ok(Self {
            entity,
            version: version_of(doc),
            payload,
            updated_at: doc
                .get("modified")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// The strong `ETag` of this generation: the quoted version number.
    #[must_use]
    pub fn etag(&self) -> String {
        format!("\"{}\"", self.version)
    }
}

/// The generation of a stored document, defaulting to the first publish when the
/// field is absent (a row written before `version` existed).
fn version_of(doc: &Value) -> u64 {
    doc.get("version").and_then(Value::as_u64).unwrap_or(1)
}

/// What a publish did, so the caller can pick a status and an `ETag`.
#[derive(Debug, Clone, PartialEq)]
pub enum PublishOutcome {
    /// The state was written; `created` distinguishes insert from update.
    Published {
        /// The freshly stored state.
        state: DesiredState,
        /// `true` when this was the entity's first publish.
        created: bool,
    },
    /// `expected_version` did not match the stored generation.
    VersionConflict {
        /// The generation actually stored (`0` when the entity is absent).
        current_version: u64,
    },
}

/// Persistent access to the per-entity desired state.
pub struct DesiredStateStore;

impl DesiredStateStore {
    /// Read one entity's desired state, or `None` when it was never published.
    ///
    /// # Errors
    ///
    /// Returns the engine permission error when the caller may not read, or the
    /// storage error of the lookup.
    pub async fn get(pool: &Db, entity: &str) -> Result<Option<DesiredState>> {
        match DocStore::get(pool, DESIRED_STATE, entity).await {
            Ok(doc) => Ok(Some(DesiredState::from_doc(&doc)?)),
            Err(Error::Document(DocumentError::NotFound { .. })) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Publish `payload` for `entity`, bumping the generation by one.
    ///
    /// When `expected_version` is `Some`, the publish is a compare-and-set: a
    /// mismatch — including a publish that expected an absent entity but found
    /// one, or expected one but found none — is reported as
    /// [`PublishOutcome::VersionConflict`] rather than overwriting a newer
    /// generation.
    ///
    /// The compare-and-set is **not atomic**: the read below and
    /// the write that follows it are two statements, so two concurrent publishes
    /// that both pass the version check can overwrite one another and the later
    /// write wins. Closing that window needs a single conditional write
    /// (`UPDATE … WHERE version = ?`) the engine does not expose yet, so callers
    /// that must not lose a concurrent publish have to serialise themselves.
    /// Documented rather than "fixed" with a table-level lock, which would cost
    /// more than the narrow race it removes.
    ///
    /// # Errors
    ///
    /// Returns a validation error for a hostile entity id, the engine
    /// permission/validation error, or the storage error of the read/write.
    pub async fn publish(
        pool: &Db,
        entity: &str,
        payload: Value,
        expected_version: Option<u64>,
    ) -> Result<PublishOutcome> {
        // The entity id becomes the document name, so it obeys the same gates a
        // route parameter does (M7): no slashes, control characters, blanks or
        // over-long names.
        validate_name(entity)?;
        // Stored as JSON text so any JSON value survives the `Code` round trip,
        // not just the object/array a `JSON` field would accept.
        let encoded = serde_json::to_string(&payload.to_json())?;
        let existing = match DocStore::get(pool, DESIRED_STATE, entity).await {
            Ok(doc) => Some(doc),
            Err(Error::Document(DocumentError::NotFound { .. })) => None,
            Err(e) => return Err(e),
        };
        let current_version = existing.as_ref().map_or(0, version_of);
        if expected_version.is_some_and(|expected| expected != current_version) {
            return Ok(PublishOutcome::VersionConflict { current_version });
        }
        let next = current_version.saturating_add(1);
        let body = ferris_core::value!({ "payload": encoded, "version": next });
        let stored = if existing.is_some() {
            DocStore::update(pool, DESIRED_STATE, entity, body).await?
        } else {
            let mut body = body;
            body.set("name".to_string(), Value::from(entity));
            DocStore::create(pool, DESIRED_STATE, body).await?
        };
        Ok(PublishOutcome::Published {
            state: DesiredState::from_doc(&stored)?,
            created: current_version == 0,
        })
    }

    /// Remove one entity's desired state. Absent is not an error: clearing is
    /// idempotent.
    ///
    /// # Errors
    ///
    /// Returns the engine permission error when the caller may not delete, or
    /// the storage error of the delete.
    pub async fn clear(pool: &Db, entity: &str) -> Result<()> {
        validate_name(entity)?;
        match DocStore::delete(pool, DESIRED_STATE, entity).await {
            Ok(()) | Err(Error::Document(DocumentError::NotFound { .. })) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Every published entity, newest first.
    ///
    /// # Errors
    ///
    /// Returns the engine permission error or the storage error of the list.
    pub async fn list(pool: &Db) -> Result<Vec<DesiredState>> {
        let params = ListParams {
            limit: Some(LIST_LIMIT),
            order_by: Some("modified desc".to_string()),
            ..Default::default()
        };
        let page = DocStore::list(pool, DESIRED_STATE, &params).await?;
        page.data.iter().map(DesiredState::from_doc).collect()
    }
}

/// Body of a publish: `{ "payload": <json>, "expected_version": <optional> }`.
#[derive(Debug, Deserialize)]
pub(crate) struct PublishRequest {
    /// The desired configuration document; any JSON value.
    pub payload: Value,
    /// Optional compare-and-set guard against the current generation.
    #[serde(default)]
    pub expected_version: Option<u64>,
}

/// `GET /api/desired-state/{entity}` — the pull half.
///
/// Answers the current state with its `ETag`, or `304 Not Modified` when
/// `If-None-Match` matches the stored version, or `404` when the entity has no
/// desired state yet.
pub(crate) async fn pull_one(
    State(state): State<Arc<ApiState>>,
    Path(entity): Path<String>,
    headers: HeaderMap,
) -> Response {
    match DesiredStateStore::get(&state.pool, &entity).await {
        Ok(None) => error_response(Error::Document(DocumentError::NotFound {
            doctype: DESIRED_STATE.to_string(),
            name: entity.clone(),
        })),
        Ok(Some(desired)) => {
            let etag = desired.etag();
            if is_not_modified(&headers, desired.version) {
                not_modified(&etag)
            } else {
                tagged_json(StatusCode::OK, &desired, &etag)
            }
        }
        Err(e) => error_response(e),
    }
}

/// `PUT /api/desired-state/{entity}` — the publish half.
pub(crate) async fn publish_one(
    State(state): State<Arc<ApiState>>,
    Path(entity): Path<String>,
    Json(request): Json<PublishRequest>,
) -> Response {
    let outcome = DesiredStateStore::publish(
        &state.pool,
        &entity,
        request.payload,
        request.expected_version,
    )
    .await;
    match outcome {
        Ok(PublishOutcome::Published { state, created }) => {
            let status = if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            tagged_json(status, &state, &state.etag())
        }
        Ok(PublishOutcome::VersionConflict { current_version }) => {
            let body = serde_json::json!({
                "error": "version_conflict",
                "message": "expected_version did not match the stored generation",
                "current_version": current_version,
            });
            (StatusCode::CONFLICT, Json(body)).into_response()
        }
        Err(e) => error_response(e),
    }
}

/// `DELETE /api/desired-state/{entity}` — clear one entity idempotently.
pub(crate) async fn clear_one(
    State(state): State<Arc<ApiState>>,
    Path(entity): Path<String>,
) -> Response {
    match DesiredStateStore::clear(&state.pool, &entity).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /api/desired-state` — every published entity, newest first.
pub(crate) async fn list_all(State(state): State<Arc<ApiState>>) -> Response {
    match DesiredStateStore::list(&state.pool).await {
        Ok(states) => Json(states).into_response(),
        Err(e) => error_response(e),
    }
}

/// Whether the request's `If-None-Match` names the stored `version`.
///
/// Accepts the three shapes a validator may carry — `"3"`, `W/"3"` and a bare
/// `3` — plus `*`, and treats the header as the comma-separated list it is.
#[must_use]
fn is_not_modified(headers: &HeaderMap, version: u64) -> bool {
    let Some(raw) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    raw.split(',').any(|token| {
        let token = token.trim();
        token == "*" || token.trim_start_matches("W/").trim_matches('"') == version.to_string()
    })
}

/// A JSON response tagged with the generation's `ETag` and `no-cache`.
fn tagged_json(status: StatusCode, value: &DesiredState, etag: &str) -> Response {
    let mut response = (status, Json(value)).into_response();
    insert_etag(&mut response, etag);
    response
}

/// The `304` answer to a matching `If-None-Match`.
fn not_modified(etag: &str) -> Response {
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    insert_etag(&mut response, etag);
    response
}

/// Attach the `ETag` and the `Cache-Control: no-cache` that makes a conditional
/// poll revalidate every time.
fn insert_etag(response: &mut Response, etag: &str) {
    if let Ok(value) = HeaderValue::from_str(etag) {
        response.headers_mut().insert(header::ETAG, value);
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin() -> ferris_auth::User {
        let mut user =
            ferris_auth::User::system_user("admin@local".to_string(), "Admin".to_string());
        user.roles = vec!["System Manager".to_string()];
        user
    }

    async fn pool() -> Db {
        crate::doctypes::register_doctypes();
        ferris_db::memory_db().await.unwrap()
    }

    #[tokio::test]
    async fn publish_creates_then_bumps_the_generation_and_round_trips_the_payload() {
        let pool = pool().await;
        ferris_document::with_user(Some(admin()), async {
            assert!(
                DesiredStateStore::get(&pool, "edge-1")
                    .await
                    .unwrap()
                    .is_none(),
                "a fresh entity has no desired state"
            );

            let first = DesiredStateStore::publish(
                &pool,
                "edge-1",
                ferris_core::value!({ "speed": 10 }),
                None,
            )
            .await
            .unwrap();
            let PublishOutcome::Published { state, created } = first else {
                panic!("a first publish must succeed: {first:?}");
            };
            assert!(created, "the first publish creates the row");
            assert_eq!(state.version, 1);
            assert_eq!(state.payload.get("speed").and_then(Value::as_i64), Some(10));
            assert!(state.updated_at.is_some(), "the engine stamps `modified`");

            let second = DesiredStateStore::publish(
                &pool,
                "edge-1",
                ferris_core::value!({ "speed": 20 }),
                None,
            )
            .await
            .unwrap();
            let PublishOutcome::Published { state, created } = second else {
                panic!("a second publish must succeed: {second:?}");
            };
            assert!(!created, "a second publish updates in place");
            assert_eq!(state.version, 2, "every publish bumps the generation");

            let pulled = DesiredStateStore::get(&pool, "edge-1")
                .await
                .unwrap()
                .expect("published");
            assert_eq!(pulled.version, 2);
            assert_eq!(
                pulled.payload.get("speed").and_then(Value::as_i64),
                Some(20)
            );
        })
        .await;
    }

    #[tokio::test]
    async fn expected_version_is_a_compare_and_set() {
        let pool = pool().await;
        ferris_document::with_user(Some(admin()), async {
            // Expecting version 0 accepts the first publish.
            let created = DesiredStateStore::publish(&pool, "edge-2", Value::from("v1"), Some(0))
                .await
                .unwrap();
            assert!(matches!(
                created,
                PublishOutcome::Published { created: true, .. }
            ));

            // A stale writer is refused, and told what the generation now is.
            let stale = DesiredStateStore::publish(&pool, "edge-2", Value::from("v2"), Some(0))
                .await
                .unwrap();
            assert_eq!(
                stale,
                PublishOutcome::VersionConflict { current_version: 1 }
            );

            // The matching generation wins.
            let updated = DesiredStateStore::publish(&pool, "edge-2", Value::from("v2"), Some(1))
                .await
                .unwrap();
            assert!(matches!(
                updated,
                PublishOutcome::Published { created: false, .. }
            ));

            // Expecting a present entity that is absent is a conflict too.
            let absent = DesiredStateStore::publish(&pool, "edge-3", Value::from("v1"), Some(4))
                .await
                .unwrap();
            assert_eq!(
                absent,
                PublishOutcome::VersionConflict { current_version: 0 }
            );
        })
        .await;
    }

    #[tokio::test]
    async fn clear_is_idempotent() {
        let pool = pool().await;
        ferris_document::with_user(Some(admin()), async {
            DesiredStateStore::publish(&pool, "edge-4", Value::from(1), None)
                .await
                .unwrap();
            DesiredStateStore::clear(&pool, "edge-4").await.unwrap();
            assert!(
                DesiredStateStore::get(&pool, "edge-4")
                    .await
                    .unwrap()
                    .is_none(),
                "cleared"
            );
            // Clearing again is a no-op, not an error.
            DesiredStateStore::clear(&pool, "edge-4").await.unwrap();
        })
        .await;
    }

    /// The payload is stored as text, so values a `JSON` field would reject —
    /// strings, numbers, bools — survive unchanged.
    #[tokio::test]
    async fn any_json_payload_value_round_trips() {
        let pool = pool().await;
        ferris_document::with_user(Some(admin()), async {
            for (entity, payload) in [
                ("scalar-string", Value::from("dry-run")),
                ("scalar-number", Value::from(42)),
                ("scalar-bool", Value::Bool(true)),
                ("an-array", ferris_core::value!([1, 2, 3])),
                ("an-object", ferris_core::value!({ "nested": { "a": 1 } })),
            ] {
                DesiredStateStore::publish(&pool, entity, payload.clone(), None)
                    .await
                    .unwrap();
                let pulled = DesiredStateStore::get(&pool, entity)
                    .await
                    .unwrap()
                    .expect("published");
                assert_eq!(pulled.payload, payload, "payload changed for {entity}");
            }
        })
        .await;
    }

    #[tokio::test]
    async fn a_hostile_entity_id_is_refused() {
        let pool = pool().await;
        ferris_document::with_user(Some(admin()), async {
            let err = DesiredStateStore::publish(&pool, "../escape", Value::from(1), None)
                .await
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::Validation(ferris_core::ValidationError::Invalid(_))
                ),
                "a slash in the entity must be refused: {err:?}"
            );
        })
        .await;
    }

    #[test]
    fn if_none_match_accepts_every_validator_shape() {
        let header = |raw: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::IF_NONE_MATCH, HeaderValue::from_str(raw).unwrap());
            headers
        };
        assert!(is_not_modified(&header("\"3\""), 3));
        assert!(is_not_modified(&header("W/\"3\""), 3));
        assert!(is_not_modified(&header("3"), 3));
        assert!(is_not_modified(&header("\"2\", \"3\""), 3));
        assert!(is_not_modified(&header("*"), 3));
        assert!(!is_not_modified(&header("\"2\""), 3));
        assert!(!is_not_modified(&HeaderMap::new(), 3));
    }
}
