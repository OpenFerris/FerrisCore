//! Write a DocType's customization into a plugin's world.
//!
//! The world is the app's own directory (`plugins/<app>/`): an exported
//! customization travels with the plugin instead of living only in the
//! database, and `ferris migrate` reads the files back
//! (`ferris-cli`'s `commands::plugin_world`). The app name is validated by
//! [`ferris_core::plugins`] before it becomes a path segment, and the write is
//! confined to `customizations/` inside that world.
//!
//! An export may be *scoped* ([`ExportScope`]): the Desk sends the fields and
//! property setters the user just changed, so the world file is a diff of that
//! edit instead of a snapshot of every customization on the DocType.

use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::response::{IntoResponse, Response};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::{Error, FileError, Result, ValidationError};
use ferris_db::Db;
use ferris_schema::contract::customization::{ExportScope, WorldCustomization};
use serde::Deserialize;

use crate::ApiState;

use super::{ensure_can_customize, read_customization};

/// The body of `POST /api/customization/{doctype}/export`.
#[derive(Debug, Deserialize)]
pub struct ExportRequest {
    /// The plugin (app) whose world receives the customization.
    pub app: String,
    /// Restrict the export to the changes the caller just made. Absent (or
    /// `null`) writes the whole customization, which is what a full export
    /// means.
    #[serde(default)]
    pub scope: Option<ExportScope>,
}

/// Write this DocType's customization into a plugin's world.
///
/// The file is `plugins/<app>/customizations/<doctype>.json`. It carries the
/// same fields and properties as the API payload plus the `DocType` itself, so
/// the world can be read back (`ferris migrate`) without guessing the name from
/// the sanitized file stem.
/// The app name is validated by `ferris_core::plugins` before it becomes a path;
/// an unknown app simply gets its world created.
///
/// `scope` narrows what is written to the changes the caller selected (see
/// [`ExportScope`]): a scoped export is a reviewable diff, a `None` export is
/// the whole customization. Either way the file keeps the complete wire shape
/// of [`ferris_schema::contract::customization::Customization`] — an empty
/// half is an empty array, not a missing key.
///
/// # Errors
///
/// A permission error (`Manage` on the DocType), a validation error for an
/// unsafe app name or for a scope that selects nothing, or a storage error
/// while writing the world.
pub async fn export_customization(
    pool: &Db,
    user: &ferris_auth::User,
    doctype: &str,
    app: &str,
    scope: Option<&ExportScope>,
) -> Result<String> {
    export_customization_at(
        pool,
        user,
        doctype,
        app,
        scope,
        &ferris_core::plugins::plugins_root(),
    )
    .await
}

/// [`export_customization`] against an explicit plugins directory.
///
/// The `plugins/` root comes from the process environment in production; a
/// caller that knows the project can pass it explicitly, which is also what
/// lets a test export into a temporary directory instead of the checkout.
///
/// # Errors
///
/// Same as [`export_customization`].
pub async fn export_customization_at(
    pool: &Db,
    user: &ferris_auth::User,
    doctype: &str,
    app: &str,
    scope: Option<&ExportScope>,
    plugins: &std::path::Path,
) -> Result<String> {
    ensure_can_customize(user, doctype)?;
    let customization = read_customization(pool, user, doctype).await?;
    let mut payload = WorldCustomization {
        doctype: doctype.to_string(),
        customization,
    };
    // Scope before touching the filesystem: a scope that selects nothing must
    // not leave a world directory behind on its way to failing.
    if let Some(scope) = scope {
        restrict_to_scope(&mut payload, scope)?;
    }
    let world = ferris_core::plugins::ensure_world_at(plugins, app)?;
    let relative =
        std::path::Path::new("customizations").join(format!("{}.json", file_stem(doctype)));
    let path = ferris_core::plugins::world_path(&world, &relative)?;
    let json = serde_json::to_string_pretty(&payload)
        .map_err(|e| Error::File(FileError::Storage(format!("cannot serialize: {e}"))))?;
    tokio::fs::write(&path, json).await.map_err(|e| {
        Error::File(FileError::Storage(format!(
            "cannot write {}: {e}",
            path.display()
        )))
    })?;
    Ok(format!(
        "plugins/{}/customizations/{}.json",
        app.trim(),
        file_stem(doctype)
    ))
}

/// Keep only what `scope` selects, dropping the other half.
///
/// A scope that matches nothing is a validation error rather than an empty
/// file: an empty `customizations/<doctype>.json` reads as "this DocType has
/// no customization" and `ferris migrate` would apply nothing, silently
/// discarding the change the user asked to export.
fn restrict_to_scope(payload: &mut WorldCustomization, scope: &ExportScope) -> Result<()> {
    let doctype = payload.doctype.clone();
    match scope {
        ExportScope::Fields(fieldnames) => {
            payload
                .customization
                .fields
                .retain(|field| fieldnames.contains(&field.fieldname));
            payload.customization.properties.clear();
            if payload.customization.fields.is_empty() {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "the export scope names no custom field of `{doctype}`"
                ))));
            }
        }
        ExportScope::Properties(targets) => {
            payload.customization.properties.retain(|setter| {
                targets.iter().any(|(fieldname, property)| {
                    setter.fieldname == *fieldname && setter.property == *property
                })
            });
            payload.customization.fields.clear();
            if payload.customization.properties.is_empty() {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "the export scope names no property setter of `{doctype}`"
                ))));
            }
        }
    }
    Ok(())
}

/// A DocType name as a file name (`Task Line` → `Task-Line`).
fn file_stem(doctype: &str) -> String {
    let stem: String = doctype
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = stem.trim_matches('-');
    if trimmed.is_empty() {
        "doctype".to_string()
    } else {
        trimmed.to_string()
    }
}

/// `POST /api/customization/{doctype}/export` — write the customization into a
/// plugin world.
pub async fn export<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path(doctype): Path<String>,
    Json(request): Json<ExportRequest>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    match export_customization(
        &state.pool,
        &user.0,
        &doctype,
        &request.app,
        request.scope.as_ref(),
    )
    .await
    {
        Ok(path) => Json(serde_json::json!({ "path": path })).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}
