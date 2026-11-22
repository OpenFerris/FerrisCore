//! Runtime metadata customization: Custom Fields and Property Setters.
//!
//! Mirrors Frappe's `Custom Field` / `Property Setter` doctypes. Customizations
//! are persisted (`tab_custom_field`, `tab_property_setter`) and applied to a
//! [`DoctypeMeta`] to produce the *effective* metadata used by validation, forms
//! and the runtime document engine — no recompilation needed.

use ferris_core::{Error, Result, SchemaError, ValidationError, Value};
use ferris_db::{
    Db, Row, Stmt,
    sql::{Column, Ident, UpsertBuilder, UpsertSet},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{
    OnceLock, RwLock,
    atomic::{AtomicU64, Ordering},
};

mod properties;
mod view_config;

use crate::registry::{DoctypeMeta, FieldMeta};
pub use properties::MAX_EXPANDED_LEVELS;
use properties::{apply_doctype_property, apply_field_order, apply_field_property};
use view_config::validate_doctype_view_property;

const CUSTOM_FIELD: &str = "tab_custom_field";
const PROPERTY_SETTER: &str = "tab_property_setter";

/// One cached effective metadata document.
struct CachedMeta {
    /// Per-DocType generation, bumped by every customization write for it.
    version: u64,
    /// Process-wide generation, bumped when a write cannot name its `DocType`
    /// (a `Custom DocPerm` row, for example).
    epoch: u64,
    meta: DoctypeMeta,
}

/// Effective metadata, keyed by `(Db::id, doctype)`.
///
/// `effective_meta` used to run three DDL statements, two `SELECT`s and a full
/// `tabCustomDocPerm` scan on **every** call; a single form open pays that six
/// to eight times. The merge is pure once the stored rows are known, so the
/// result is cached per database and invalidated by the writers below
/// (PERF-03/SCH-04).
static META_CACHE: OnceLock<RwLock<HashMap<(u64, String), CachedMeta>>> = OnceLock::new();

/// Per-DocType customization generation.
static META_VERSIONS: OnceLock<RwLock<HashMap<String, u64>>> = OnceLock::new();

/// Writers that cannot name a single `DocType` bump this instead.
static META_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Databases whose customization tables have already been created in this
/// process. The DDL moved off the read path; boot and `ferris migrate` still
/// run [`CustomizationStore::init`], and this set is the one-time fallback for
/// a process (a test, an embedder) that reaches a read first.
static META_INITIALISED: OnceLock<RwLock<HashSet<u64>>> = OnceLock::new();

#[cfg(test)]
static META_LOADS: OnceLock<RwLock<HashMap<String, u64>>> = OnceLock::new();

fn meta_cache() -> &'static RwLock<HashMap<(u64, String), CachedMeta>> {
    META_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn meta_versions() -> &'static RwLock<HashMap<String, u64>> {
    META_VERSIONS.get_or_init(|| RwLock::new(HashMap::new()))
}

fn meta_initialised() -> &'static RwLock<HashSet<u64>> {
    META_INITIALISED.get_or_init(|| RwLock::new(HashSet::new()))
}

/// The current `(Doctype generation, process epoch)` pair a read must match.
pub(crate) fn meta_generation(doctype: &str) -> (u64, u64) {
    let version = meta_versions()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(doctype)
        .copied()
        .unwrap_or(0);
    (version, META_EPOCH.load(Ordering::Acquire))
}

/// The cached effective metadata for `doctype`, when it is still current.
pub(crate) fn cached_meta(pool_id: u64, doctype: &str) -> Option<DoctypeMeta> {
    let (version, epoch) = meta_generation(doctype);
    let cached = meta_cache()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&(pool_id, doctype.to_string()))
        .filter(|entry| entry.version == version && entry.epoch == epoch)
        .map(|entry| entry.meta.clone());
    #[cfg(test)]
    if cached.is_none() {
        *META_LOADS
            .get_or_init(|| RwLock::new(HashMap::new()))
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(doctype.to_string())
            .or_insert(0) += 1;
    }
    cached
}

/// Store `meta` for `(pool_id, doctype)` under the generation read *before* the
/// load. A write that happened while the load ran bumps the generation, so the
/// entry is already stale and the next read reloads instead of serving it.
pub(crate) fn cache_meta(pool_id: u64, doctype: &str, generation: (u64, u64), meta: &DoctypeMeta) {
    meta_cache()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            (pool_id, doctype.to_string()),
            CachedMeta {
                version: generation.0,
                epoch: generation.1,
                meta: meta.clone(),
            },
        );
}

/// Create the customization tables once per database handle.
// `Error` is the crate-wide type; boxing it would leak into every caller.
#[allow(clippy::result_large_err)]
pub(crate) async fn ensure_tables(pool: &Db) -> Result<()> {
    if meta_initialised()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&pool.id())
    {
        return Ok(());
    }
    CustomizationStore::with_db(pool.clone()).init().await?;
    meta_initialised()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(pool.id());
    Ok(())
}

#[cfg(test)]
pub(crate) fn meta_loads(doctype: &str) -> u64 {
    META_LOADS
        .get_or_init(|| RwLock::new(HashMap::new()))
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(doctype)
        .copied()
        .unwrap_or(0)
}

fn custom_field_table() -> Vec<Column> {
    vec![
        Column::new("doctype", "VARCHAR(255)")
            .not_null()
            .primary_key(),
        Column::new("fieldname", "VARCHAR(255)")
            .not_null()
            .primary_key(),
        Column::new("label", "VARCHAR(255)"),
        Column::new("fieldtype", "VARCHAR(64)").not_null(),
        Column::new("options", "TEXT"),
        Column::new("insert_after", "VARCHAR(255)"),
        Column::new("reqd", "INTEGER").not_null().default("0"),
        // `default` is a reserved word: quoted per dialect, a bare `"default"`
        // is a string literal on MySQL.
        Column::new("default", "TEXT"),
        Column::new("hidden", "INTEGER").not_null().default("0"),
        Column::new("read_only", "INTEGER").not_null().default("0"),
    ]
}

fn property_setter_table() -> Vec<Column> {
    vec![
        Column::new("id", "INTEGER").auto_increment(),
        Column::new("doctype", "VARCHAR(255)").not_null(),
        Column::new("fieldname", "VARCHAR(255)"),
        Column::new("property", "VARCHAR(64)").not_null(),
        Column::new("value", "TEXT").not_null(),
    ]
}

/// Quote a compile-time-known identifier for the dialect in play.
fn q(dialect: ferris_db::Dialect, name: &str) -> Result<String> {
    Ok(Ident::new(name)?.quote(dialect))
}

/// A field added at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomField {
    pub doctype: String,
    pub fieldname: String,
    #[serde(default)]
    pub label: Option<String>,
    pub fieldtype: String,
    #[serde(default)]
    pub options: Option<String>,
    /// Insert the field after this fieldname (else append).
    #[serde(default)]
    pub insert_after: Option<String>,
    #[serde(default)]
    pub reqd: bool,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub read_only: bool,
}

/// A property override for a `DocType` or one of its fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertySetter {
    pub doctype: String,
    /// `None` targets a `DocType`-level property.
    #[serde(default)]
    pub fieldname: Option<String>,
    pub property: String,
    pub value: String,
}

/// Persisted customizations.
#[derive(Default)]
pub struct CustomizationStore {
    db: Option<Db>,
    custom_fields: RwLock<Vec<CustomField>>,
    property_setters: RwLock<Vec<PropertySetter>>,
}

impl CustomizationStore {
    /// Invalidate the cached effective metadata of `doctype`.
    ///
    /// Called by every customization writer, so the next `effective_meta`
    /// reloads the stored rows instead of serving a stale merge. Exposed
    /// because a writer that stores a `Custom Field`/`Property Setter` row
    /// directly (the Desk's generic document path, a fixture import) must be
    /// able to declare the same fact.
    pub fn invalidate_meta(doctype: &str) {
        *meta_versions()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(doctype.to_string())
            .or_insert(0) += 1;
        meta_cache()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(_, name), _| name != doctype);
    }

    /// Invalidate every cached effective metadata document.
    ///
    /// Used when a write cannot name the `DocType` it affects — a
    /// `Custom DocPerm` row carries its parent inside the document body, and
    /// update/delete callers do not always have it at hand.
    pub fn invalidate_all_meta() {
        META_EPOCH.fetch_add(1, Ordering::AcqRel);
        meta_cache()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_db(db: Db) -> Self {
        Self {
            db: Some(db),
            ..Default::default()
        }
    }

    /// Create the backing tables.
    pub async fn init(&self) -> Result<()> {
        let Some(db) = &self.db else {
            return Ok(());
        };
        db.ensure_table(CUSTOM_FIELD, &custom_field_table()).await?;
        db.ensure_table(PROPERTY_SETTER, &property_setter_table())
            .await?;
        db.ensure_index(
            "idx_property_setter_doctype",
            PROPERTY_SETTER,
            &["doctype"],
            false,
        )
        .await
    }

    /// Add (or replace) a custom field.
    ///
    /// # Errors
    ///
    /// Rejects an empty doctype/fieldname and an **unknown fieldtype**
    /// (`Error::InvalidFieldType`): the allowlist in
    /// [`crate::fieldtype`] is what the Desk renderer, the validation
    /// dispatch and the WIT/TS generators are keyed on, so a name outside it
    /// would silently degrade to a plain text input. The value is stored in
    /// its canonical spelling.
    pub async fn add_custom_field(&self, field: CustomField) -> Result<()> {
        if field.doctype.trim().is_empty() || field.fieldname.trim().is_empty() {
            return Err(Error::Validation(ValidationError::Required(
                "custom field doctype and fieldname".to_string(),
            )));
        }
        let mut field = field;
        let Some(canonical) = crate::fieldtype::canonical(&field.fieldtype) else {
            return Err(Error::Schema(SchemaError::InvalidFieldType(
                field.fieldtype.clone(),
            )));
        };
        field.fieldtype = canonical.to_string();
        let doctype = field.doctype.clone();
        if let Some(db) = &self.db {
            let label = Value::from(field.label.clone());
            let fieldtype = Value::from(field.fieldtype.as_str());
            let options = Value::from(field.options.clone());
            let insert_after = Value::from(field.insert_after.clone());
            let reqd = Value::from(i64::from(field.reqd));
            let default = Value::from(field.default.clone());
            let hidden = Value::from(i64::from(field.hidden));
            let read_only = Value::from(i64::from(field.read_only));
            db.upsert(
                UpsertBuilder::new(db.dialect(), CUSTOM_FIELD)
                    .values(vec![
                        ("doctype", Value::from(field.doctype.as_str())),
                        ("fieldname", Value::from(field.fieldname.as_str())),
                        ("label", label.clone()),
                        ("fieldtype", fieldtype.clone()),
                        ("options", options.clone()),
                        ("insert_after", insert_after.clone()),
                        ("reqd", reqd.clone()),
                        ("default", default.clone()),
                        ("hidden", hidden.clone()),
                        ("read_only", read_only.clone()),
                    ])
                    .conflict_on(["doctype", "fieldname"])
                    .on_update("label", UpsertSet::Value(label))
                    .on_update("fieldtype", UpsertSet::Value(fieldtype))
                    .on_update("options", UpsertSet::Value(options))
                    .on_update("insert_after", UpsertSet::Value(insert_after))
                    .on_update("reqd", UpsertSet::Value(reqd))
                    .on_update("default", UpsertSet::Value(default))
                    .on_update("hidden", UpsertSet::Value(hidden))
                    .on_update("read_only", UpsertSet::Value(read_only))
                    .build()?,
            )
            .await?;
        }
        let mut fields = self
            .custom_fields
            .write()
            .unwrap_or_else(|e| e.into_inner());
        fields.retain(|f| !(f.doctype == field.doctype && f.fieldname == field.fieldname));
        fields.push(field);
        drop(fields);
        Self::invalidate_meta(&doctype);
        Ok(())
    }

    /// Add a property setter.
    ///
    /// A setter that targets `fieldname` with property `fieldtype` goes through
    /// the same allowlist as [`Self::add_custom_field`]: an unknown fieldtype
    /// would be applied verbatim to the effective metadata, where the Desk
    /// renderer and validation dispatch would silently fall back to plain text.
    /// The canonical spelling and the canonical property name are stored, so
    /// the setter cannot be accepted and then ignored by `apply`.
    ///
    /// # Errors
    ///
    /// Rejects an empty doctype/property and an unknown fieldtype
    /// ([`Error::InvalidFieldType`]).
    pub async fn set_property(&self, setter: PropertySetter) -> Result<()> {
        if setter.doctype.trim().is_empty() || setter.property.trim().is_empty() {
            return Err(Error::Validation(ValidationError::Required(
                "property setter doctype and property".to_string(),
            )));
        }
        let mut setter = setter;
        let doctype = setter.doctype.clone();
        if setter.fieldname.is_some() && setter.property.trim().eq_ignore_ascii_case("fieldtype") {
            let Some(canonical) = crate::fieldtype::canonical(&setter.value) else {
                return Err(Error::Schema(SchemaError::InvalidFieldType(
                    setter.value.clone(),
                )));
            };
            setter.property = "fieldtype".to_string();
            setter.value = canonical.to_string();
        }
        // The collection-view properties name fields of the doctype, so an
        // unknown field or a wrong fieldtype must be refused at the write
        // boundary: `apply` would otherwise store a setter no renderer can
        // honour, and the Desk would show an empty view with no explanation.
        //
        // Only doctypes the registry knows can be checked. A store used
        // without a registry (unit tests, an app whose declaration has not
        // landed yet) accepts the setter; the renderer still reports an
        // unresolved field instead of pretending the configuration worked.
        if setter.fieldname.is_none()
            && let Some(base) = crate::get_doctype(&setter.doctype)
        {
            let effective = self.apply(&base).await?;
            validate_doctype_view_property(
                &base.name,
                &effective,
                &setter.property,
                &setter.value,
            )?;
        }
        // A property has one value: replace the previous row instead of
        // appending. The table is append-only and `apply` honours the last
        // insert, so every re-edit used to leave a dead row behind — visible in
        // the exported fixture and in the advanced dialog, and deleted as a
        // group by the undo endpoint.
        self.delete_property_setter(
            &setter.doctype,
            setter.fieldname.as_deref(),
            &setter.property,
        )
        .await?;
        if let Some(db) = &self.db {
            // The `id` is assigned by the database. Computing `MAX(id) + 1` here
            // would race two concurrent setters onto the same value.
            db.execute(&Stmt::new(
                format!(
                    "INSERT INTO {} (doctype, fieldname, property, value) VALUES (?, ?, ?, ?)",
                    q(db.dialect(), PROPERTY_SETTER)?
                ),
                vec![
                    Value::from(setter.doctype.as_str()),
                    Value::from(setter.fieldname.clone()),
                    Value::from(setter.property.as_str()),
                    Value::from(setter.value.as_str()),
                ],
            ))
            .await?;
        }
        self.property_setters
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(setter);
        Self::invalidate_meta(&doctype);
        Ok(())
    }

    /// Remove a custom field. Removing a field that is not customized deletes
    /// nothing and succeeds, so undoing a customization twice is safe.
    ///
    /// Removing a *compiled* field's customization would restore the field to
    /// its registry definition, not delete the field: only rows in
    /// `tab_custom_field` are removed.
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table or deleting the row.
    pub async fn delete_custom_field(&self, doctype: &str, fieldname: &str) -> Result<()> {
        if let Some(db) = &self.db {
            db.execute(&Stmt::new(
                format!(
                    "DELETE FROM {} WHERE doctype = ? AND fieldname = ?",
                    q(db.dialect(), CUSTOM_FIELD)?
                ),
                vec![Value::from(doctype), Value::from(fieldname)],
            ))
            .await?;
        }
        self.custom_fields
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|f| !(f.doctype == doctype && f.fieldname == fieldname));
        Self::invalidate_meta(doctype);
        Ok(())
    }

    /// Remove property setters of a `DocType`.
    ///
    /// A setter is identified by `(doctype, fieldname, property)`; `fieldname`
    /// is optional and `None` targets the `DocType`-level setters. Removing
    /// matches deletes nothing and succeeds.
    ///
    /// # Errors
    ///
    /// Returns the storage error of creating the table or deleting the rows.
    pub async fn delete_property_setter(
        &self,
        doctype: &str,
        fieldname: Option<&str>,
        property: &str,
    ) -> Result<()> {
        if let Some(db) = &self.db {
            // `fieldname IS NULL` rather than `= NULL`, which matches nothing in
            // every dialect: a DocType-level setter has a NULL `fieldname`.
            let sql = match fieldname {
                Some(_) => format!(
                    "DELETE FROM {} WHERE doctype = ? AND fieldname = ? AND property = ?",
                    q(db.dialect(), PROPERTY_SETTER)?
                ),
                None => format!(
                    "DELETE FROM {} WHERE doctype = ? AND fieldname IS NULL AND property = ?",
                    q(db.dialect(), PROPERTY_SETTER)?
                ),
            };
            let params = fieldname.map_or_else(
                || vec![Value::from(doctype), Value::from(property)],
                |fieldname| {
                    vec![
                        Value::from(doctype),
                        Value::from(fieldname),
                        Value::from(property),
                    ]
                },
            );
            db.execute(&Stmt::new(sql, params)).await?;
        }
        self.property_setters
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|s| {
                !(s.doctype == doctype
                    && s.fieldname.as_deref() == fieldname
                    && s.property == property)
            });
        Self::invalidate_meta(doctype);
        Ok(())
    }

    /// Custom fields for a `DocType`.
    pub async fn custom_fields(&self, doctype: &str) -> Result<Vec<CustomField>> {
        if let Some(db) = &self.db {
            let rows = db
                .fetch_all(&Stmt::new(
                    format!(
                        "SELECT * FROM {} WHERE doctype = ? ORDER BY fieldname",
                        q(db.dialect(), CUSTOM_FIELD)?
                    ),
                    vec![Value::from(doctype)],
                ))
                .await?;
            Ok(rows.iter().map(row_to_custom_field).collect())
        } else {
            Ok(self
                .custom_fields
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter(|f| f.doctype == doctype)
                .cloned()
                .collect())
        }
    }

    /// Property setters for a `DocType`.
    pub async fn property_setters(&self, doctype: &str) -> Result<Vec<PropertySetter>> {
        if let Some(db) = &self.db {
            let rows = db
                .fetch_all(&Stmt::new(
                    format!(
                        "SELECT * FROM {} WHERE doctype = ? ORDER BY id",
                        q(db.dialect(), PROPERTY_SETTER)?
                    ),
                    vec![Value::from(doctype)],
                ))
                .await?;
            Ok(rows
                .iter()
                .map(|row| PropertySetter {
                    doctype: row.str("doctype").unwrap_or_default(),
                    fieldname: row.opt_str("fieldname"),
                    property: row.str("property").unwrap_or_default(),
                    value: row.str("value").unwrap_or_default(),
                })
                .collect())
        } else {
            Ok(self
                .property_setters
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter(|s| s.doctype == doctype)
                .cloned()
                .collect())
        }
    }

    /// Apply custom fields and property setters to produce effective metadata.
    pub async fn apply(&self, meta: &DoctypeMeta) -> Result<DoctypeMeta> {
        let mut effective = meta.clone();

        for field in self.custom_fields(&meta.name).await? {
            if effective
                .fields
                .iter()
                .any(|f| f.fieldname == field.fieldname)
            {
                continue;
            }
            let field_meta = FieldMeta {
                fieldname: field.fieldname.clone(),
                label: field
                    .label
                    .clone()
                    .or_else(|| Some(field.fieldname.clone())),
                fieldtype: field.fieldtype.clone(),
                options: field.options.clone(),
                default: field.default.clone(),
                reqd: field.reqd,
                read_only: field.read_only,
                hidden: field.hidden,
                ..Default::default()
            };
            let position = field
                .insert_after
                .as_deref()
                .and_then(|after| {
                    effective
                        .fields
                        .iter()
                        .position(|f| f.fieldname == after)
                        .map(|index| index + 1)
                })
                .unwrap_or(effective.fields.len());
            effective.fields.insert(position, field_meta);
        }

        let setters = self.property_setters(&meta.name).await?;
        for setter in &setters {
            match &setter.fieldname {
                Some(fieldname) => {
                    if let Some(field) = effective
                        .fields
                        .iter_mut()
                        .find(|f| &f.fieldname == fieldname)
                    {
                        apply_field_property(field, &setter.property, &setter.value);
                    }
                }
                None => apply_doctype_property(&mut effective, &setter.property, &setter.value),
            }
        }

        // Order last: moving a field must not change which field a value setter
        // landed on, and it lets the Desk move compiled fields (which have no
        // `insert_after` of their own) across sections and tabs.
        apply_field_order(&mut effective.fields, &setters);

        Ok(effective)
    }
}

fn row_to_custom_field(row: &Row) -> CustomField {
    CustomField {
        doctype: row.str("doctype").unwrap_or_default(),
        fieldname: row.str("fieldname").unwrap_or_default(),
        label: row.opt_str("label"),
        fieldtype: row.str("fieldtype").unwrap_or_default(),
        options: row.opt_str("options"),
        insert_after: row.opt_str("insert_after"),
        reqd: row.opt_bool("reqd").unwrap_or(false),
        default: row.opt_str("default"),
        hidden: row.opt_bool("hidden").unwrap_or(false),
        read_only: row.opt_bool("read_only").unwrap_or(false),
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn probe(name: &str) -> DoctypeMeta {
        DoctypeMeta::new(name, "C3")
    }

    #[tokio::test]
    async fn a_second_read_does_not_run_ddl_or_selects_again() {
        const DOCTYPE: &str = "C3CachedProbeRead";
        crate::register_doctype(probe(DOCTYPE));
        let db = ferris_db::memory_db().await.unwrap();

        let before = meta_loads(DOCTYPE);
        let first = crate::effective_meta(&db, DOCTYPE)
            .await
            .unwrap()
            .expect("the probe is registered");
        let second = crate::effective_meta(&db, DOCTYPE)
            .await
            .unwrap()
            .expect("the probe is registered");
        assert_eq!(
            meta_loads(DOCTYPE) - before,
            1,
            "the second read is a cache hit"
        );
        assert_eq!(first.fields.len(), second.fields.len());
    }

    #[tokio::test]
    async fn a_write_invalidates_the_cached_metadata() {
        const DOCTYPE: &str = "C3CachedProbeWrite";
        crate::register_doctype(probe(DOCTYPE));
        let db = ferris_db::memory_db().await.unwrap();
        CustomizationStore::with_db(db.clone())
            .init()
            .await
            .unwrap();
        let _ = crate::effective_meta(&db, DOCTYPE).await.unwrap();
        let before = meta_loads(DOCTYPE);

        CustomizationStore::with_db(db.clone())
            .add_custom_field(CustomField {
                doctype: DOCTYPE.to_string(),
                fieldname: "probe_field".to_string(),
                label: None,
                fieldtype: "Data".to_string(),
                options: None,
                insert_after: None,
                reqd: false,
                default: None,
                hidden: false,
                read_only: false,
            })
            .await
            .unwrap();

        let meta = crate::effective_meta(&db, DOCTYPE)
            .await
            .unwrap()
            .expect("the probe is registered");
        assert!(
            meta.fields.iter().any(|f| f.fieldname == "probe_field"),
            "the write must be visible on the next read"
        );
        assert_eq!(
            meta_loads(DOCTYPE) - before,
            1,
            "the write forced one reload"
        );

        CustomizationStore::invalidate_meta(DOCTYPE);
        let _ = crate::effective_meta(&db, DOCTYPE).await.unwrap();
        assert_eq!(
            meta_loads(DOCTYPE) - before,
            2,
            "the explicit invalidation forces another reload"
        );
    }
}

#[cfg(test)]
mod tests;
