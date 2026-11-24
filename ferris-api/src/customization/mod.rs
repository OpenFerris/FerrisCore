//! Runtime metadata customization ("Customize Form"): the logic and the HTTP
//! endpoints.
//!
//! These read and write the persisted customizations that
//! `ferris_schema::CustomizationStore` applies to a `DocType`'s effective
//! metadata: custom fields (`tab_custom_field`) and property setters
//! (`tab_property_setter`). A change here is visible on the next request without
//! a recompilation, which is what "customizable at runtime, persisted on the
//! server" means.
//!
//! The gate and the store work live in the `read_*`/`save_*`/`remove_*`
//! functions, not in the axum handlers: the SSR bridge (`ferris-web`'s
//! `DocStoreApi`) calls the same functions in-process, so SSR and hydrate cannot
//! diverge on who may customize or on what a change means. The handlers are thin
//! wrappers that add the HTTP status code.
//!
//! Every operation requires `Manage` on the `DocType`, resolved through the
//! permission engine ([`ferris_core::permissions::check_permission`]) rather
//! than a hardcoded role list, so the "who may customize" policy lives with the
//! rest of the policy. In practice that is the System Manager, matching Frappe's
//! own Customize Form right.
//!
//! `fieldtype` is the one exception, and it is deliberately narrower: it
//! decides whether a value is hashed, encrypted or stored in the clear, so a
//! setter that changes it requires `System Manager` **and** may never turn a
//! `Password`/`Secret` field back into a plain one (SCH-02/SEC-13/API-24). A
//! `Manage` grant over a `DocType` is a management right, not a licence to
//! strip a compiled credential field's semantics and read it in the clear on
//! the next request.
//!
//! [`export`] writes the same payload into a plugin's world
//! (`plugins/<app>/customizations/<doctype>.json`); the split into
//! `export.rs` keeps this file to the database surface.

use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::permissions::{Permission, Resource, SYSTEM_MANAGER_ROLE, check_permission};
use ferris_core::{AuthzError, Error, Result, ValidationError};
use ferris_db::Db;
use ferris_schema::CustomizationStore;
use ferris_schema::contract::customization::{CustomFieldSpec, Customization, PropertySetterSpec};
use ferris_schema::customization::{CustomField, PropertySetter};
use serde::Deserialize;

use crate::ApiState;

/// Authorize a customization operation as the caller.
///
/// A `DocType`'s customization is a management operation on the `DocType`, so it is
/// checked as `Manage` over a `DocType`-level resource (empty name): the same
/// resource type the engine uses for the documents themselves.
///
/// # Errors
///
/// Returns `PermissionDenied` when the caller may not manage the `DocType`.
pub fn ensure_can_customize(user: &ferris_auth::User, doctype: &str) -> Result<()> {
    let resource = Resource::new(doctype, "");
    if check_permission(user, &resource, Permission::Manage) {
        return Ok(());
    }
    Err(Error::Authz(AuthzError::PermissionDenied {
        action: Permission::Manage.as_str().to_string(),
        resource: format!("{doctype} customization"),
    }))
}

/// Open the customization store and make sure its tables exist.
async fn store_with(pool: &Db) -> Result<CustomizationStore> {
    let store = CustomizationStore::with_db(pool.clone());
    store.init().await?;
    Ok(store)
}

/// A fieldtype whose stored value is a credential: hashed (`Password`) or
/// encrypted (`Secret`). Both are masked on read by the document engine.
fn is_sensitive_fieldtype(fieldtype: &str) -> bool {
    matches!(fieldtype.trim(), "Password" | "Secret")
}

/// Authorize a `fieldtype` change, which is not an ordinary presentation edit.
///
/// Two rules, both fail-closed:
///
/// * only `System Manager` may change a `fieldtype` through a property setter,
///   whatever `Manage` the caller holds on the `DocType`;
/// * a sensitive compiled field may be retyped only to another sensitive type.
///   `Password → Data` would stop the hash on write and the mask on read, so the
///   next read would return the stored hash, and `Secret → Data` would hand out
///   the ciphertext — both are credential disclosures dressed as a layout edit.
///
/// A setter whose field is not in the compiled metadata is untouched here: there
/// is no compiled semantics to degrade, and a custom field's own type is
/// validated by [`ensure_custom_field_type_change`].
fn ensure_fieldtype_change(
    user: &ferris_auth::User,
    doctype: &str,
    spec: &PropertySetterSpec,
) -> Result<()> {
    if spec.property.trim() != "fieldtype" {
        return Ok(());
    }
    let Some(fieldname) = spec.fieldname.as_deref().filter(|name| !name.is_empty()) else {
        return Err(Error::Validation(ValidationError::Invalid(
            "a `fieldtype` property setter must name a field".to_string(),
        )));
    };
    if !user.has_role(SYSTEM_MANAGER_ROLE) {
        return Err(Error::Authz(AuthzError::PermissionDenied {
            action: "manage_fieldtype".to_string(),
            resource: format!("{doctype}.{fieldname}"),
        }));
    }
    let Some(meta) = ferris_schema::get_doctype(doctype) else {
        return Err(Error::Schema(ferris_core::SchemaError::DocTypeNotFound(
            doctype.to_string(),
        )));
    };
    let Some(field) = meta
        .fields
        .iter()
        .find(|field| field.fieldname == fieldname)
    else {
        return Ok(());
    };
    if is_sensitive_fieldtype(&field.fieldtype) && !is_sensitive_fieldtype(&spec.value) {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "`{fieldname}` is a `{}` field; a `{}` value cannot be stored by a `{}` field",
            field.fieldtype,
            field.fieldtype,
            spec.value.trim()
        ))));
    }
    Ok(())
}

/// Authorize a `fieldtype` change on an existing **custom** field.
///
/// The same rule as [`ensure_fieldtype_change`], applied to the row already in
/// [`CustomizationStore`]: changing a declared type is a System Manager action,
/// and a sensitive one is never downgraded.
async fn ensure_custom_field_type_change(
    store: &CustomizationStore,
    user: &ferris_auth::User,
    doctype: &str,
    spec: &CustomFieldSpec,
) -> Result<()> {
    let Some(existing) = store
        .custom_fields(doctype)
        .await?
        .into_iter()
        .find(|field| field.fieldname == spec.fieldname)
    else {
        return Ok(());
    };
    if existing.fieldtype == spec.fieldtype {
        return Ok(());
    }
    if !user.has_role(SYSTEM_MANAGER_ROLE) {
        return Err(Error::Authz(AuthzError::PermissionDenied {
            action: "manage_fieldtype".to_string(),
            resource: format!("{doctype}.{}", spec.fieldname),
        }));
    }
    if is_sensitive_fieldtype(&existing.fieldtype) && !is_sensitive_fieldtype(&spec.fieldtype) {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "`{}` is a `{}` field and cannot become a `{}` one",
            spec.fieldname, existing.fieldtype, spec.fieldtype
        ))));
    }
    Ok(())
}

/// The customizations currently persisted for `doctype`.
///
/// # Errors
///
/// Returns a validation error for a malformed doctype, `PermissionDenied` when
/// the caller may not manage it, and the storage error of reading the rows.
pub async fn read_customization(
    pool: &Db,
    user: &ferris_auth::User,
    doctype: &str,
) -> Result<Customization> {
    crate::store::validate_doctype(doctype)?;
    ensure_can_customize(user, doctype)?;
    let store = store_with(pool).await?;
    let fields = store.custom_fields(doctype).await?;
    let properties = store.property_setters(doctype).await?;
    Ok(Customization {
        fields: fields.into_iter().map(field_to_spec).collect(),
        properties: properties.into_iter().map(setter_to_spec).collect(),
    })
}

/// Add or replace a custom field, returning the stored spec.
///
/// # Errors
///
/// Returns a validation error for a malformed doctype or an incomplete field,
/// `PermissionDenied` when the caller may not manage the `DocType`, and the
/// storage error of writing the row.
pub async fn save_custom_field(
    pool: &Db,
    user: &ferris_auth::User,
    doctype: &str,
    spec: &CustomFieldSpec,
) -> Result<CustomFieldSpec> {
    crate::store::validate_doctype(doctype)?;
    ensure_can_customize(user, doctype)?;
    let store = store_with(pool).await?;
    ensure_custom_field_type_change(&store, user, doctype, spec).await?;
    store
        .add_custom_field(CustomField {
            doctype: doctype.to_string(),
            fieldname: spec.fieldname.clone(),
            label: spec.label.clone(),
            fieldtype: spec.fieldtype.clone(),
            options: spec.options.clone(),
            insert_after: spec.insert_after.clone(),
            reqd: spec.reqd,
            default: spec.default.clone(),
            hidden: spec.hidden,
            read_only: spec.read_only,
        })
        .await?;
    Ok(spec.clone())
}

/// Undo a custom field.
///
/// # Errors
///
/// Returns a validation error for a malformed doctype, `PermissionDenied` when
/// the caller may not manage the `DocType`, and the storage error of deleting.
pub async fn remove_custom_field(
    pool: &Db,
    user: &ferris_auth::User,
    doctype: &str,
    fieldname: &str,
) -> Result<()> {
    crate::store::validate_doctype(doctype)?;
    ensure_can_customize(user, doctype)?;
    let store = store_with(pool).await?;
    // The field row and its setters are separate tables. Deleting only the row
    // left the setters behind: they would reappear if a field with the same
    // name was created again, and they leaked into a whole-DocType export.
    let existing = read_customization(pool, user, doctype).await?;
    for setter in existing
        .properties
        .iter()
        .filter(|setter| setter.fieldname.as_deref() == Some(fieldname))
    {
        store
            .delete_property_setter(doctype, Some(fieldname), &setter.property)
            .await?;
    }
    store.delete_custom_field(doctype, fieldname).await
}

/// Add or replace a property setter, returning the stored spec.
///
/// # Errors
///
/// Returns a validation error for a malformed doctype, `PermissionDenied` when
/// the caller may not manage the `DocType`, and the storage error of writing.
pub async fn save_property_setter(
    pool: &Db,
    user: &ferris_auth::User,
    doctype: &str,
    spec: &PropertySetterSpec,
) -> Result<PropertySetterSpec> {
    crate::store::validate_doctype(doctype)?;
    ensure_can_customize(user, doctype)?;
    ensure_fieldtype_change(user, doctype, spec)?;
    // `permlevel` hides a field from callers and `mask` redacts its value: a
    // site manager may edit the form, but not the access policy.
    if matches!(spec.property.as_str(), "permlevel" | "mask") && !user.has_role(SYSTEM_MANAGER_ROLE)
    {
        return Err(Error::Authz(AuthzError::PermissionDenied {
            action: format!("set `{}`", spec.property),
            resource: format!("doctype `{doctype}`"),
        }));
    }
    // A position that names no field is accepted by the store and ignored by
    // the merge; rejecting it here turns a silent no-op into an answer.
    if spec.property == "insert_after"
        && let (Some(fieldname), anchor) = (spec.fieldname.as_deref(), spec.value.trim())
        && !anchor.is_empty()
        && anchor != "__first__"
        && anchor != fieldname
    {
        let meta = crate::store::effective_meta(pool, doctype)
            .await?
            .ok_or_else(|| {
                Error::Schema(ferris_core::SchemaError::DocTypeNotFound(
                    doctype.to_string(),
                ))
            })?;
        if !meta.fields.iter().any(|field| field.fieldname == anchor) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "insert_after: `{anchor}` is not a field of `{doctype}`"
            ))));
        }
    }
    store_with(pool)
        .await?
        .set_property(PropertySetter {
            doctype: doctype.to_string(),
            fieldname: spec.fieldname.clone(),
            property: spec.property.clone(),
            value: spec.value.clone(),
        })
        .await?;
    Ok(spec.clone())
}

/// Undo a property setter, `fieldname` scoping it to a field (`None` for a
/// `DocType`-level setter).
///
/// # Errors
///
/// Returns a validation error for a malformed doctype, `PermissionDenied` when
/// the caller may not manage the `DocType`, and the storage error of deleting.
pub async fn remove_property_setter(
    pool: &Db,
    user: &ferris_auth::User,
    doctype: &str,
    fieldname: Option<&str>,
    property: &str,
) -> Result<()> {
    crate::store::validate_doctype(doctype)?;
    ensure_can_customize(user, doctype)?;
    store_with(pool)
        .await?
        .delete_property_setter(doctype, fieldname, property)
        .await
}

/// `GET /api/customization/{doctype}` — the current customizations.
pub async fn get_customization<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path(doctype): Path<String>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    match read_customization(&state.pool, &user.0, &doctype).await {
        Ok(customization) => Json(customization).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `POST /api/customization/{doctype}/fields` — add or replace a custom field.
pub async fn put_custom_field<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path(doctype): Path<String>,
    Json(spec): Json<CustomFieldSpec>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    match save_custom_field(&state.pool, &user.0, &doctype, &spec).await {
        Ok(saved) => (StatusCode::CREATED, Json(saved)).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `DELETE /api/customization/{doctype}/fields/{fieldname}` — undo a custom
/// field.
pub async fn delete_custom_field<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((doctype, fieldname)): Path<(String, String)>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    match remove_custom_field(&state.pool, &user.0, &doctype, &fieldname).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `POST /api/customization/{doctype}/properties` — add or replace a property
/// setter.
pub async fn put_property_setter<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path(doctype): Path<String>,
    Json(spec): Json<PropertySetterSpec>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    match save_property_setter(&state.pool, &user.0, &doctype, &spec).await {
        Ok(saved) => (StatusCode::CREATED, Json(saved)).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// Query for the property-setter delete: the optional owning field.
#[derive(Debug, Default, Deserialize)]
pub struct PropertyQuery {
    #[serde(default)]
    fieldname: Option<String>,
}

/// `DELETE /api/customization/{doctype}/properties/{property}` — undo a property
/// setter. `?fieldname=` scopes it to a field; without it the `DocType`-level
/// setter is removed.
pub async fn delete_property_setter<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((doctype, property)): Path<(String, String)>,
    Query(query): Query<PropertyQuery>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    match remove_property_setter(
        &state.pool,
        &user.0,
        &doctype,
        query.fieldname.as_deref(),
        &property,
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// Map the store's custom field to the wire spec (dropping the `DocType`, which
/// the URL already names).
fn field_to_spec(field: CustomField) -> CustomFieldSpec {
    CustomFieldSpec {
        fieldname: field.fieldname,
        label: field.label,
        fieldtype: field.fieldtype,
        options: field.options,
        insert_after: field.insert_after,
        reqd: field.reqd,
        default: field.default,
        hidden: field.hidden,
        read_only: field.read_only,
    }
}

/// Map the store's property setter to the wire spec.
fn setter_to_spec(setter: PropertySetter) -> PropertySetterSpec {
    PropertySetterSpec {
        fieldname: setter.fieldname,
        property: setter.property,
        value: setter.value,
    }
}

mod export;
#[cfg(test)]
mod tests;

pub use export::{ExportRequest, export, export_customization, export_customization_at};

#[cfg(test)]
mod fieldtype_tests {
    use super::*;
    use ferris_core::permissions::{
        PermissionEngine, Policy, PolicyStatement, Principal, Resource, global_engine_ref,
        set_global_engine,
    };

    /// The permission engine is process-wide; serialise the test that swaps it.
    static SERIALISE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Restore {
        _serialised: std::sync::MutexGuard<'static, ()>,
        previous: PermissionEngine,
    }

    impl Drop for Restore {
        fn drop(&mut self) {
            set_global_engine(self.previous.clone());
        }
    }

    fn user(roles: &[&str]) -> ferris_auth::User {
        let mut user =
            ferris_auth::User::system_user("probe@local".to_string(), "Probe".to_string());
        user.id = "probe@local".to_string();
        user.roles = roles.iter().map(|role| (*role).to_string()).collect();
        user
    }

    /// A `Manage` grant for `role` over `doctype`, installed as the global
    /// engine for the duration of the test.
    fn manage_scope(doctype: &str, role: &str) -> Restore {
        let serialised = SERIALISE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = global_engine_ref()
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut engine = PermissionEngine::default();
        engine.add_policy(Policy::new(vec![PolicyStatement::allow(
            vec![Principal::role(role)],
            vec![Permission::Manage],
            vec![Resource::new(doctype, "*"), Resource::new(doctype, "")],
        )]));
        set_global_engine(engine);
        Restore {
            _serialised: serialised,
            previous,
        }
    }

    fn password_probe(doctype: &str) {
        ferris_schema::register_doctype(ferris_schema::DoctypeMeta {
            name: doctype.to_string(),
            module: "test".to_string(),
            fields: vec![ferris_schema::FieldMeta {
                fieldname: "api_secret".to_string(),
                fieldtype: "Password".to_string(),
                ..Default::default()
            }],
            hidden: true,
            ..Default::default()
        });
    }

    /// SCH-02/SEC-13/API-24: `Manage` on a DocType does not include rewriting
    /// the type of one of its compiled fields.
    #[tokio::test]
    async fn a_manage_role_cannot_change_a_fieldtype() {
        let doctype = "B2 Fieldtype Manage Doc";
        let _restore = manage_scope(doctype, "RWD");
        password_probe(doctype);
        let pool = ferris_db::memory_db().await.unwrap();

        let err = save_property_setter(
            &pool,
            &user(&["RWD"]),
            doctype,
            &PropertySetterSpec {
                fieldname: Some("api_secret".to_string()),
                property: "fieldtype".to_string(),
                value: "Data".to_string(),
            },
        )
        .await
        .expect_err("a Manage holder must not retype a compiled field");
        assert!(
            matches!(err, Error::Authz(AuthzError::PermissionDenied { .. })),
            "{err:?}"
        );
    }

    /// The same rule is not lifted by being System Manager: a `Password` field
    /// is hashed on write and masked on read, and `Data` would stop both, so
    /// the downgrade is refused whatever the caller's role.
    #[tokio::test]
    async fn a_system_manager_cannot_downgrade_a_password_field() {
        let doctype = "B2 Fieldtype Admin Doc";
        password_probe(doctype);
        let pool = ferris_db::memory_db().await.unwrap();

        let err = save_property_setter(
            &pool,
            &user(&["System Manager"]),
            doctype,
            &PropertySetterSpec {
                fieldname: Some("api_secret".to_string()),
                property: "fieldtype".to_string(),
                value: "Data".to_string(),
            },
        )
        .await
        .expect_err("a Password field must never become a plain one");
        assert!(matches!(err, Error::Validation(_)), "{err:?}");

        // Retyping it to another sensitive type is a deliberate operator
        // decision and is accepted (the store write itself is what we assert).
        save_property_setter(
            &pool,
            &user(&["System Manager"]),
            doctype,
            &PropertySetterSpec {
                fieldname: Some("api_secret".to_string()),
                property: "fieldtype".to_string(),
                value: "Secret".to_string(),
            },
        )
        .await
        .expect("a sensitive-to-sensitive retype is allowed");
    }
}
