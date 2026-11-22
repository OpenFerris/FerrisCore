//! `GET|PUT /api/dashboard-layout/doc/{doctype}/{name}` — the layout of a
//! document that **is** its own dashboard (the `document` binding of
//! [`ferris_schema::contract::DashboardView`]).
//!
//! The routes in [`crate::dashboard_layout`] answer for an app's
//! `(owner, id)` pair: the shared row a dashboard's widgets and board config
//! are saved in. A document-format dashboard has no such row — the doctype's
//! bound `widgets` field *is* the layout and its bound `config` field is the
//! board override, both shared by everyone who opens that document — so this
//! module reads and writes the document itself, through the engine's ordinary
//! permission policy:
//!
//! - **the gate is the declaration**: only a doctype that declares
//!   `view: dashboard` *and* the `document` binding answers here, and every
//!   field the format names must exist in the doctype (a binding to a missing
//!   field would read and write nothing). The `config` binding is the one
//!   optional name: unbound means the document has no board override;
//! - **`editable` is `Write` on the document**
//!   ([`crate::store::ensure_writable`]), not `Create` on a `Dashboard Layout`
//!   row, because the write *is* a write of the document;
//! - **there is nothing to restore**: the document is the state, so
//!   `customized` stays `false`, the Desk's "Restaurar app" never applies and
//!   this module registers no `DELETE` — an attempt answers `405` instead of
//!   pretending to restore a seed that never existed;
//! - **saving is an ordinary document write** (`DocStore::update`): versioned,
//!   logged and emitting the doctype's own events, with the widget list
//!   filtered by the very [`crate::dashboard_layout::sanitize_widgets`] the
//!   app-dashboard path uses, under [`crate::dashboards::OWNER`] — the owner
//!   the Desk's grid mounts with, so the two sides agree on which references
//!   are valid. The board config is clamped by
//!   [`ferris_schema::contract::DashboardConfig::clamped`], the same rule the
//!   app-dashboard path applies.

use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::response::{IntoResponse, Response};

use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::{Error, Result, ValidationError, Value};
use ferris_db::Db;
use ferris_schema::DoctypeMeta;
use ferris_schema::contract::{DashboardLayout, DashboardLayoutPayload, DocView, DocumentFormat};

use crate::ApiState;
use crate::dashboard_layout::{normalize_widgets, sanitize_widgets};
use crate::dashboards::OWNER;
use crate::store::{DocStore, effective_meta, ensure_writable};

/// A rejected request in the validation bucket: everything this endpoint
/// refuses is a declaration that does not hold, not a storage fault.
const fn invalid(message: String) -> Error {
    Error::Validation(ValidationError::Invalid(message))
}

/// The format `meta`'s documents carry their dashboard in, or the reason this
/// doctype has no document dashboard at all.
///
/// The three **layout** field names the format declares are checked here, once,
/// for both handlers: a missing `widgets` field is a binding that would write
/// into nothing, and a missing `layout`/`label` field would silently fall back
/// to defaults while the endpoint kept answering — one loud failure instead.
///
/// `config` is the one **optional** binding: a doctype that names it must
/// declare the field (a binding that would write into nothing is refused the
/// same way), but a doctype that leaves it unbound has no board override at
/// all, so nothing is checked and the layout endpoint answers with the Desk's
/// global default. That is what lets every existing document dashboard keep
/// working without adding a field it never asked for.
#[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
fn format_of(meta: &DoctypeMeta) -> Result<DocumentFormat> {
    if meta.view != DocView::Dashboard {
        return Err(invalid(format!(
            "`{}` does not open its documents as a dashboard",
            meta.name
        )));
    }
    let Some(format) = meta.dashboard.document.clone() else {
        return Err(invalid(format!(
            "`{}` does not declare the document dashboard binding",
            meta.name
        )));
    };
    let mut bound = vec![format.widgets(), format.layout(), format.label()];
    // Only the *declared* config field is required to exist: leaving the
    // binding unbound is the documented "no board override" case.
    if format.config.is_some() {
        bound.push(format.config());
    }
    for field in bound {
        if !meta
            .fields
            .iter()
            .any(|declared| declared.fieldname == field)
        {
            return Err(invalid(format!(
                "`{}` binds its dashboard to field `{field}`, which the doctype does not declare",
                meta.name
            )));
        }
    }
    Ok(format)
}

/// The declared format behind the route, with the doctype declaration that
/// carries it: the doctype must exist (through [`effective_meta`]) and hold the
/// binding ([`format_of`]).
#[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
async fn binding(pool: &Db, doctype: &str) -> Result<(DoctypeMeta, DocumentFormat)> {
    let meta = effective_meta(pool, doctype)
        .await?
        .ok_or_else(|| invalid(format!("no DocType named `{doctype}`")))?;
    let format = format_of(&meta)?;
    Ok((meta, format))
}

/// `GET /api/dashboard-layout/doc/{doctype}/{name}` — the document's own layout
/// and board configuration.
///
/// The bound `widgets` field itself, normalized exactly like a seed, plus the
/// board override from the bound `config` field (or none when the doctype never
/// bound one), so the editor draws a shape the next save can reproduce.
///
/// `editable` is `Write` on *this* document; `customized` is always `false`:
/// there is no declared seed behind a document dashboard, so the arrangement
/// on show is the document itself — nothing to be personalizing. The
/// `AuthenticatedUser` extractor is only the session gate (401 before any
/// declaration is read): who may *see* the document is the engine's `Read`
/// policy, evaluated on the read below and never restated here.
pub async fn get_document_layout<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    _user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    let (_, format) = match binding(&state.pool, &doctype).await {
        Ok(binding) => binding,
        Err(e) => return crate::router::error_response(e),
    };
    let doc = match DocStore::get(&state.pool, &doctype, &name).await {
        Ok(doc) => doc,
        Err(e) => return crate::router::error_response(e),
    };
    let data = doc.to_json();

    let mut widgets = format.widgets_in(&data);
    normalize_widgets(OWNER, &mut widgets);

    // The editor flag: a denial means read-only, any other failure is
    // answered as itself — a read-only view must never hide a broken store.
    let editable = match ensure_writable(&state.pool, &doctype, &name).await {
        Ok(()) => true,
        Err(Error::Authz(_)) => false,
        Err(e) => return crate::router::error_response(e),
    };

    Json(DashboardLayout {
        editable,
        customized: false,
        widgets,
        config: format.config_in(&data).unwrap_or_default(),
    })
    .into_response()
}

/// `PUT /api/dashboard-layout/doc/{doctype}/{name}` — save a document
/// dashboard's layout and board configuration.
///
/// The widgets go into the bound `widgets` field and the board override into
/// the bound `config` field (when the doctype declares one). The write goes
/// through `DocStore::update` under the ambient session, so it is an ordinary
/// document write: the same `Write` policy as any other edit, version history,
/// the activity log and the doctype's own events. The answer carries what was
/// actually saved (unresolvable references dropped, spans clamped, config
/// clamped) — with `customized: false`, because the document *is* the state and
/// nothing was personalized. Like `GET`, the extractor is the session gate; the
/// engine owns the authorization.
pub async fn put_document_layout<S>(
    State(state): State<std::sync::Arc<ApiState>>,
    _user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
    Json(body): Json<DashboardLayoutPayload>,
) -> Response
where
    std::sync::Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    let (meta, format) = match binding(&state.pool, &doctype).await {
        Ok(binding) => binding,
        Err(e) => return crate::router::error_response(e),
    };
    // `Write` on the document, decided by the engine: a denial stops the
    // request here, and a missing document answers 404 rather than a save
    // that went nowhere.
    if let Err(e) = ensure_writable(&state.pool, &doctype, &name).await {
        return crate::router::error_response(e);
    }

    let effective = sanitize_widgets(OWNER, &body.widgets);
    let stored = serde_json::to_string(&effective).unwrap_or_else(|_| "[]".to_string());
    // The config is clamped like the geometry, so a hand-crafted payload cannot
    // store a padding that breaks the board. A doctype that never bound the
    // config field simply has nowhere to keep it — its override is discarded,
    // matching what its `GET` reports (no override at all).
    let config = body.config.clamped();

    let mut fields = serde_json::Map::new();
    fields.insert(
        format.widgets().to_string(),
        serde_json::Value::String(stored),
    );
    if format.config.is_some() {
        fields.insert(
            format.config().to_string(),
            config_as_stored(&meta, format.config(), &config),
        );
    }
    let data = Value::from_json(serde_json::Value::Object(fields));

    match DocStore::update(&state.pool, &doctype, &name, data).await {
        Ok(_) => Json(DashboardLayout {
            editable: true,
            customized: false,
            widgets: effective,
            config,
        })
        .into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// The board override in the shape the bound field can hold.
///
/// A `JSON` field stores the object itself (its validator accepts objects and
/// arrays only, so a serialized string would be refused); every other type
/// (`Code`, `Data`, `Text`) stores the serialized string, the same shape the
/// `widgets` list uses. [`DocumentFormat::config_in`] reads both, so the
/// declaration — not the endpoint — decides the storage shape.
fn config_as_stored(
    meta: &DoctypeMeta,
    field: &str,
    config: &ferris_schema::contract::DashboardConfig,
) -> serde_json::Value {
    let json_field = meta
        .fields
        .iter()
        .find(|declared| declared.fieldname == field)
        .is_some_and(|declared| declared.fieldtype == "JSON");
    if json_field {
        serde_json::to_value(config).unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::String(
            serde_json::to_string(config).unwrap_or_else(|_| "{}".to_string()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_schema::FieldMeta;
    use ferris_schema::contract::{DashboardConfig, DashboardView};

    /// A doctype declaration shaped for the gate: the view kind, the binding
    /// and the field names the format may name.
    fn meta(view: DocView, dashboard: DashboardView, fields: &[&str]) -> DoctypeMeta {
        DoctypeMeta {
            name: "Panel".to_string(),
            view,
            dashboard,
            fields: fields
                .iter()
                .map(|name| FieldMeta {
                    fieldname: name.to_string(),
                    ..FieldMeta::default()
                })
                .collect(),
            ..DoctypeMeta::default()
        }
    }

    /// The route answers only for a doctype that both opens its documents as
    /// a dashboard *and* binds the document: a `Dashboard` row (native
    /// binding) is served by `/api/dashboard-layout/db/{id}` instead, and any
    /// other view is not a dashboard at all.
    #[test]
    fn the_gate_requires_the_dashboard_view_and_the_document_binding() {
        let declared = meta(
            DocView::Dashboard,
            DashboardView::document(),
            &["widgets", "layout", "label"],
        );
        let format = format_of(&declared).expect("the declared binding");
        assert_eq!(format, DocumentFormat::default());

        let native = meta(DocView::Dashboard, DashboardView::default(), &["widgets"]);
        assert!(
            format_of(&native).is_err(),
            "a Dashboard row has no binding"
        );

        let form = meta(DocView::Form, DashboardView::document(), &["widgets"]);
        assert!(format_of(&form).is_err(), "a form is not a dashboard");
    }

    /// A binding that names a field the doctype does not declare is refused
    /// loudly (validation error, not a dashboard that quietly writes nothing),
    /// and the refusal names the field that broke it.
    #[test]
    fn a_binding_to_a_missing_field_is_refused() {
        let broken = meta(
            DocView::Dashboard,
            DashboardView::document_fields(DocumentFormat::default().with_widgets("mis_widgets")),
            &["widgets", "layout", "label"],
        );
        match format_of(&broken) {
            Err(Error::Validation(ValidationError::Invalid(message))) => {
                assert!(
                    message.contains("mis_widgets"),
                    "the refusal names the missing field: {message}"
                );
            }
            other => panic!("expected a validation error naming the field, got {other:?}"),
        }
    }

    /// Per-field renames ride the gate intact: the endpoint reads and writes
    /// the fields the declaration named, never the canonical ones.
    #[test]
    fn a_field_rename_is_honoured() {
        let renamed = meta(
            DocView::Dashboard,
            DashboardView::document_fields(
                DocumentFormat::default()
                    .with_widgets("panel_widgets")
                    .with_layout("panel_layout")
                    .with_label("titulo"),
            ),
            &["panel_widgets", "panel_layout", "titulo"],
        );
        let format = format_of(&renamed).expect("all three fields exist");
        assert_eq!(format.widgets(), "panel_widgets");
        assert_eq!(format.layout(), "panel_layout");
        assert_eq!(format.label(), "titulo");
    }

    /// `config` is the one **optional** binding: a doctype that leaves it
    /// unbound has no board override and is not required to declare a field —
    /// that is what keeps every existing document dashboard working — while a
    /// doctype that *names* a config field must declare it, the same loud
    /// refusal as the widget list.
    #[test]
    fn the_config_binding_is_optional_but_checked_when_named() {
        // Unbound: the three layout fields alone are enough.
        let unbound = meta(
            DocView::Dashboard,
            DashboardView::document(),
            &["widgets", "layout", "label"],
        );
        let format = format_of(&unbound).expect("no config binding is fine");
        assert!(format.config.is_none());
        assert_eq!(format.config(), "config");

        // Named and declared: accepted.
        let bound = meta(
            DocView::Dashboard,
            DashboardView::document_fields(DocumentFormat::default().with_config("panel_config")),
            &["widgets", "layout", "label", "panel_config"],
        );
        let format = format_of(&bound).expect("the config field exists");
        assert_eq!(format.config(), "panel_config");

        // Named but missing: refused, naming the field.
        let broken = meta(
            DocView::Dashboard,
            DashboardView::document_fields(DocumentFormat::default().with_config("panel_config")),
            &["widgets", "layout", "label"],
        );
        match format_of(&broken) {
            Err(Error::Validation(ValidationError::Invalid(message))) => {
                assert!(
                    message.contains("panel_config"),
                    "the refusal names the missing field: {message}"
                );
            }
            other => panic!("expected a validation error naming the field, got {other:?}"),
        }
    }

    /// `config_in` reads the board override out of the bound field, tolerating
    /// both the string a `Code` field stores and the object a `JSON` field
    /// stores, and answers `None` (not an error) when the field is absent or
    /// the document never declared one.
    #[test]
    fn config_in_reads_both_stored_shapes() {
        let format = DocumentFormat::default().with_config("board");
        // A `JSON`-typed field stores the object itself.
        let object = serde_json::json!({ "board": { "style": { "gap_px": 20 } } });
        let config = format.config_in(&object).expect("the object shape reads");
        assert_eq!(config.style.gap_px, Some(20));

        // A `Code`-typed field stores the serialized string.
        let string = serde_json::json!({ "board": "{\"style\":{\"gap_px\":24}}" });
        let config = format.config_in(&string).expect("the string shape reads");
        assert_eq!(config.style.gap_px, Some(24));

        // Absent field, and a document with no board at all: no override, no
        // error.
        assert!(format.config_in(&serde_json::json!({})).is_none());
        assert!(
            format
                .config_in(&serde_json::json!({ "board": "not json" }))
                .is_none(),
            "a malformed value reads as no override, not as a failure"
        );
    }

    /// The write picks the shape the declaration can hold: a `JSON` field
    /// refuses a string (its validator accepts objects and arrays only), while a
    /// `Code`/`Data` field stores the serialized text — the same shape the
    /// `widgets` list uses.
    #[test]
    fn the_config_is_stored_in_the_shape_the_field_accepts() {
        let config = DashboardConfig {
            dashboard_id: None,
            style: ferris_schema::contract::DashboardStyle {
                gap_px: Some(16),
                ..Default::default()
            },
        };
        let mut declared = meta(
            DocView::Dashboard,
            DashboardView::document_fields(DocumentFormat::default().with_config("board")),
            &["widgets", "layout", "label", "board"],
        );
        let set_type = |declared: &mut DoctypeMeta, fieldtype: &str| {
            declared
                .fields
                .iter_mut()
                .find(|field| field.fieldname == "board")
                .expect("the declared config field")
                .fieldtype = fieldtype.to_string();
        };

        set_type(&mut declared, "JSON");
        let stored = config_as_stored(&declared, "board", &config);
        assert!(
            stored.is_object(),
            "a JSON field stores the object itself, got {stored:?}"
        );
        assert_eq!(stored["style"]["gap_px"], 16);

        set_type(&mut declared, "Code");
        let stored = config_as_stored(&declared, "board", &config);
        let text = stored.as_str().expect("a Code field stores text");
        let back: DashboardConfig = serde_json::from_str(text).expect("the text parses back");
        assert_eq!(back.style.gap_px, Some(16));
    }
}
