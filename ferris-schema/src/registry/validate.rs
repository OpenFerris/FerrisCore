//! Validation of app-declared DocTypes against the live registry.

use super::reserved::{RESERVED_DOCTYPES, normalize_doctype_name};
use super::store::{Origin, REGISTRY, SchemaRegistry};
use super::types::DoctypeMeta;

/// Validate the app identity used in the origin and module fields.
fn validate_app_identity(app: &str) -> ferris_core::Result<()> {
    let app = app.trim();
    if app.is_empty() {
        return Err(ferris_core::Error::Validation(
            ferris_core::ValidationError::Invalid(
                "a plugin DocType needs a non-empty app identity".to_string(),
            ),
        ));
    }
    if app.len() > 64 {
        return Err(ferris_core::Error::Validation(
            ferris_core::ValidationError::Invalid(format!(
                "app identity is longer than 64 characters: {app:?}"
            )),
        ));
    }
    Ok(())
}

impl SchemaRegistry {
    /// Validate a DocType an app wants to declare, without registering it.
    ///
    /// The host calls this for **every** entry before applying any side effect,
    /// so a descriptor cannot leave themes/workspaces/UI registered and then be
    /// refused halfway through its doctype loop. `try_register_app` re-runs the
    /// same checks against the live registry, which is what makes the final
    /// insert authoritative under concurrent registrations.
    pub fn validate_app_doctype(&self, app: &str, meta: &DoctypeMeta) -> ferris_core::Result<()> {
        validate_app_identity(app)?;
        let name = meta.name.trim();
        if name.is_empty() {
            return Err(ferris_core::Error::Validation(
                ferris_core::ValidationError::Invalid(
                    "a plugin DocType needs a non-empty name".to_string(),
                ),
            ));
        }
        validate_fields(meta)?;
        if meta.naming_type == Some(crate::contract::meta::NamingType::Field)
            && meta
                .naming_field
                .as_deref()
                .is_none_or(|field| field.trim().is_empty())
        {
            return Err(ferris_core::Error::Validation(
                ferris_core::ValidationError::Invalid(format!(
                    "DocType `{}` uses naming type `field` but declares no `naming_field`",
                    meta.name
                )),
            ));
        }
        let normalized = normalize_doctype_name(name);
        if RESERVED_DOCTYPES.contains(normalized.as_str()) {
            return Err(ferris_core::Error::Schema(
                ferris_core::SchemaError::ReservedDocType(name.to_string()),
            ));
        }

        // The physical table is what actually collides: `document_table`
        // strips whitespace, so `ToDo` and `To Do` are the same `tabToDo`.
        // Compare case-insensitively too, because MySQL treats table names that
        // way and the framework must not depend on the backend.
        let table = crate::migration::document_table(name)?;
        let table_key = table.to_ascii_lowercase();
        {
            let origins = self.origins.read().unwrap_or_else(|e| e.into_inner());
            if let Some(origin) = origins.get(name) {
                match origin {
                    Origin::Framework => {
                        return Err(ferris_core::Error::Schema(
                            ferris_core::SchemaError::ReservedDocType(name.to_string()),
                        ));
                    }
                    Origin::App(owner) if owner != app.trim() => {
                        return Err(ferris_core::Error::Schema(
                            ferris_core::SchemaError::DocTypeOwnedBy {
                                name: name.to_string(),
                                owner: owner.clone(),
                            },
                        ));
                    }
                    Origin::App(_) => {}
                }
            }
        }
        let doctypes = self.doctypes.read().unwrap_or_else(|e| e.into_inner());
        for (existing, _) in doctypes.iter() {
            if existing == name {
                continue;
            }
            let Ok(existing_table) = crate::migration::document_table(existing) else {
                continue;
            };
            if existing_table.eq_ignore_ascii_case(&table_key) {
                return Err(ferris_core::Error::Schema(
                    ferris_core::SchemaError::DocTypeTableCollision {
                        name: name.to_string(),
                        other: existing.clone(),
                        table,
                    },
                ));
            }
        }
        Ok(())
    }

    /// Register a DocType on behalf of `app`, enforcing the reserved-name and
    /// cross-app collision policy.
    ///
    /// # Errors
    ///
    /// Returns [`ferris_core::SchemaError::ReservedDocType`] for a framework
    /// name, [`ferris_core::SchemaError::DocTypeOwnedBy`] for another app's
    /// name, and [`ferris_core::SchemaError::DocTypeTableCollision`] when the
    /// name maps to a table another DocType already owns.
    pub fn try_register_app(&self, app: &str, mut meta: DoctypeMeta) -> ferris_core::Result<()> {
        self.validate_app_doctype(app, &meta)?;
        meta.name = meta.name.trim().to_string();
        // The authority is the host's `plugin.name()`, never the `module` the
        // guest wrote into its metadata: otherwise a plugin could claim another
        // app's namespace while still being refused by the origin check.
        meta.module = app.trim().to_string();
        self.insert_meta(&mut meta, Origin::App(app.trim().to_string()));
        Ok(())
    }
}

/// Validate the fields of an app-declared DocType.
///
/// The check lives here, not in the descriptor projection, so the descriptor
/// subset and `schema::declare-doctype` (raw JSON) cannot disagree about what a
/// field must be: a non-empty `fieldname`, a canonical `fieldtype`, and no two
/// fields sharing a name.
fn validate_fields(meta: &DoctypeMeta) -> ferris_core::Result<()> {
    let mut seen = std::collections::HashSet::with_capacity(meta.fields.len());
    for (index, field) in meta.fields.iter().enumerate() {
        let fieldname = field.fieldname.trim();
        if fieldname.is_empty() {
            return Err(ferris_core::Error::Validation(
                ferris_core::ValidationError::Invalid(format!(
                    "DocType `{}` field #{index} has an empty fieldname",
                    meta.name
                )),
            ));
        }
        if !seen.insert(fieldname) {
            return Err(ferris_core::Error::Validation(
                ferris_core::ValidationError::Invalid(format!(
                    "DocType `{}` declares `{fieldname}` more than once",
                    meta.name
                )),
            ));
        }
        if let Some(canonical) = crate::fieldtype::canonical(&field.fieldtype) {
            if canonical != field.fieldtype {
                // `canonical` is the spelling the renderer and the validator
                // share; storing a lookalike leaves a field no control renders.
                return Err(ferris_core::Error::Validation(
                    ferris_core::ValidationError::Invalid(format!(
                        "DocType `{}` field `{fieldname}` uses fieldtype `{}`; use the canonical `{canonical}`",
                        meta.name, field.fieldtype
                    )),
                ));
            }
        } else {
            return Err(ferris_core::Error::Validation(
                ferris_core::ValidationError::Invalid(format!(
                    "DocType `{}` field `{fieldname}` has unknown fieldtype `{}` (allowed names: ferris_schema::fieldtype::FIELDTYPES)",
                    meta.name, field.fieldtype
                )),
            ));
        }
    }
    Ok(())
}

/// Validate an app-declared DocType against the live registry without
/// registering it. The WASM host validates the whole descriptor with this before
/// applying any side effect.
///
/// # Errors
///
/// See [`SchemaRegistry::try_register_app`].
pub fn validate_app_doctype(app: &str, meta: &DoctypeMeta) -> ferris_core::Result<()> {
    REGISTRY.validate_app_doctype(app, meta)
}

/// Register a DocType on behalf of the named app, enforcing the reserved-name
/// and cross-app collision policy.
///
/// # Errors
///
/// See [`SchemaRegistry::try_register_app`].
pub fn try_register_app_doctype(app: &str, meta: DoctypeMeta) -> ferris_core::Result<()> {
    REGISTRY.try_register_app(app, meta)
}
