//! Persisting the schema registry as documents of the `DocType` DocType.
//!
//! The registry (`ferris_schema::registry`) is a process-global map built from
//! compiled declarations. This store is its data-driven mirror: it writes each
//! registered [`DoctypeMeta`] into `tabDocType` as an ordinary document, so the
//! schema can be listed and inspected through the standard document engine.
//!
//! Two directions, both idempotent:
//!
//! * [`DoctypeStore::sync`] walks the registry and upserts one row per DocType.
//!   This is what the host calls at boot, next to `seed_singletons`.
//! * [`DoctypeStore::load_into_registry`] reads those rows back and registers
//!   any DocType the compiled code did not declare (a DocType that only exists
//!   in the database — `custom = true`). Compiled declarations always win: the
//!   row is skipped when the name is already registered, so a stale row cannot
//!   shadow the code.
//!
//! The full metadata travels in the `meta_json` field, because the columns the
//! Desk lists on cannot represent `indexes`, `workflow` or the long flag tail;
//! the columns are a projection, `meta_json` is the source.
#![allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.

use ferris_core::{Error, Result, SchemaError, SerializationError, Value};
use ferris_db::Db;
use ferris_schema::registry::DoctypeMeta;

use crate::engine::{DocumentEngine, EngineContext};

use super::DOCTYPE;

/// What a [`DoctypeStore::sync`] did.
#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    /// Names of the DocTypes written (created or refreshed). In registry order.
    pub synced: Vec<String>,
    /// Names of DocTypes found in the database that compiled code did not
    /// declare, and which [`DoctypeStore::sync_registry`] therefore registered
    /// as `custom` (DOC9). Empty for a plain [`DoctypeStore::sync`], which only
    /// writes the registry into the database.
    pub loaded: Vec<String>,
    /// `name: error` lines for DocTypes that could not be written. A bad
    /// declaration must not stop the boot, so these are reported rather than
    /// returned as an error — the same contract as `seed_singletons`.
    pub warnings: Vec<String>,
}

/// Read/write access to the stored DocType metadata.
pub struct DoctypeStore<'a> {
    db: &'a Db,
}

impl<'a> DoctypeStore<'a> {
    #[must_use]
    pub const fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// The engine context for a framework metadata write.
    ///
    /// The context targets the **`DocType` table**, not `meta.name`: the row
    /// being written is a `DocType` row whose `name` (in the body) is the
    /// declared doctype. Building the context from `meta.name` sent every
    /// registered DocType to a table of its own (`tabStoreProbe`, …), so
    /// nothing ever reached `tabDocType` and [`Self::load_into_registry`] read
    /// an empty table back — the round trip DOC9 wires was never exercised.
    ///
    /// Metadata is not owned by a caller: reads and writes run as the system,
    /// with anonymous access allowed. This is the same rationale as
    /// `DocStore::get_single_system` — the framework must be able to read and
    /// materialise its own metadata before any request exists — and the write
    /// surface stays admin-only through the `DocType` DocType's own role rules
    /// whenever a real user goes through the API instead of this store.
    fn context() -> EngineContext<'static> {
        match ferris_schema::get_doctype(DOCTYPE) {
            Some(meta) => {
                EngineContext::new(DOCTYPE, meta.naming_series.clone()).allow_anonymous(true)
            }
            // `DocType` is not registered in every binary (unit tests link no
            // doctypes); the engine still serves it through the lazy table path
            // with the framework's own metadata declaration absent.
            None => EngineContext::new(DOCTYPE, "").allow_anonymous(true),
        }
    }

    /// Project a registry meta into a `DocType` document body.
    fn body(meta: &DoctypeMeta) -> Result<Value> {
        let json = serde_json::to_value(meta)
            .map_err(|e| Error::Serialization(SerializationError::Json(e)))?;
        let mut body = ferris_core::Map::new();
        body.insert("name".to_string(), Value::from(meta.name.clone()));
        body.insert("module".to_string(), Value::from(meta.module.clone()));
        body.insert(
            "description".to_string(),
            Value::from(meta.description.clone().unwrap_or_default()),
        );
        body.insert("is_single".to_string(), Value::from(meta.is_single));
        body.insert(
            "is_submittable".to_string(),
            Value::from(meta.is_submittable),
        );
        body.insert(
            "is_child_table".to_string(),
            Value::from(meta.is_child_table),
        );
        body.insert("is_tree".to_string(), Value::from(meta.is_tree));
        body.insert("is_virtual".to_string(), Value::from(meta.is_virtual));
        body.insert("custom".to_string(), Value::from(meta.custom));
        body.insert(
            "naming_series".to_string(),
            Value::from(meta.naming_series.clone()),
        );
        body.insert(
            "title_field".to_string(),
            Value::from(meta.title_field.clone().unwrap_or_default()),
        );
        body.insert(
            "field_count".to_string(),
            Value::from(i64::try_from(meta.fields.len()).unwrap_or(i64::MAX)),
        );
        body.insert("fields".to_string(), field_rows(meta));
        body.insert(
            "meta_json".to_string(),
            Value::from(
                serde_json::to_string(&json)
                    .map_err(|e| Error::Serialization(SerializationError::Json(e)))?,
            ),
        );
        Ok(Value::Object(body))
    }

    /// Rebuild a [`DoctypeMeta`] from a stored `DocType` row.
    ///
    /// Prefers `meta_json` (lossless). A row written by hand without it falls
    /// back to the projected columns, which is enough for a DocType with no
    /// indexes or permissions.
    fn meta_of(document: &Value) -> Result<DoctypeMeta> {
        if let Some(raw) = document.get("meta_json").and_then(Value::as_str)
            && !raw.is_empty()
        {
            let json: serde_json::Value = serde_json::from_str(raw)
                .map_err(|e| Error::Serialization(SerializationError::Json(e)))?;
            return serde_json::from_value(json)
                .map_err(|e| Error::Serialization(SerializationError::Json(e)));
        }
        let name = document
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut meta = DoctypeMeta::new(
            name,
            document
                .get("module")
                .and_then(Value::as_str)
                .unwrap_or("Custom"),
        );
        meta.description = document
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string);
        // The `fields` child table is the columnar projection of the field
        // list. Prefer `meta_json` above; this fallback is enough for a row
        // written by hand without it.
        if let Some(fields) = document.get("fields").and_then(Value::as_array) {
            meta.fields = fields
                .iter()
                .filter_map(|row| {
                    let fieldname = row.get("fieldname").and_then(Value::as_str)?.to_string();
                    Some(ferris_schema::FieldMeta {
                        fieldname,
                        label: row
                            .get("label")
                            .and_then(Value::as_str)
                            .filter(|label| !label.is_empty())
                            .map(str::to_string),
                        fieldtype: row
                            .get("fieldtype")
                            .and_then(Value::as_str)
                            .unwrap_or("Data")
                            .to_string(),
                        options: row
                            .get("options")
                            .and_then(Value::as_str)
                            .filter(|options| !options.is_empty())
                            .map(str::to_string),
                        default: row
                            .get("default")
                            .and_then(Value::as_str)
                            .filter(|default| !default.is_empty())
                            .map(str::to_string),
                        reqd: row.get("reqd").and_then(Value::as_bool).unwrap_or(false),
                        in_list_view: row
                            .get("in_list_view")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        ..Default::default()
                    })
                })
                .collect();
        }
        meta.custom = true;
        Ok(meta)
    }

    /// Upsert `meta` into `tabDocType`, returning its stored document.
    async fn upsert(&self, meta: &DoctypeMeta) -> Result<Value> {
        // The engine only creates a column for a field the effective metadata
        // declares. Registering the framework's `DocType`/`DocField` first
        // makes the metadata store work even when the host catalog has not run
        // (tests, an embedder). Idempotent.
        super::register_doctypes();
        let engine = DocumentEngine::new(self.db.clone());
        let ctx = Self::context();
        let body = Self::body(meta)?;
        if engine.get(&ctx, &meta.name).await.is_ok() {
            engine.update(&ctx, &meta.name, body).await
        } else {
            engine.create(&ctx, body).await
        }
    }

    /// Mirror every registered DocType into `tabDocType`.
    ///
    /// Idempotent: an existing row is updated in place, a missing one created.
    /// Never fails as a whole — a failure for one DocType is collected in
    /// [`SyncReport::warnings`] so one bad declaration cannot abort the boot.
    pub async fn sync(&self, metas: &[DoctypeMeta]) -> SyncReport {
        let mut report = SyncReport::default();
        for meta in metas {
            match self.upsert(meta).await {
                Ok(_) => report.synced.push(meta.name.clone()),
                Err(e) => report.warnings.push(format!("{}: {e}", meta.name)),
            }
        }
        report
    }

    /// Convenience wrapper over [`Self::sync`] for the process-global registry:
    /// the entry point the host calls at boot.
    ///
    /// Runs **both** directions, which the module's own docs promise
    /// (`store.rs:12-16`): compiled declarations are mirrored into `tabDocType`,
    /// then rows that only exist in the database (`custom = true`) are read back
    /// into the registry by [`Self::load_into_registry`]. Without the second half
    /// a hand-created DocType row was written by `sync` but never registered, so
    /// its documents could not be served (DOC9). A read-back failure is collected
    /// as a warning rather than aborting the boot, matching `sync`'s contract.
    pub async fn sync_registry(&self) -> SyncReport {
        let metas: Vec<DoctypeMeta> = ferris_schema::list_doctypes_metas()
            .into_iter()
            .map(|meta| (*meta).clone())
            .collect();
        let mut report = self.sync(&metas).await;
        match self.load_into_registry().await {
            Ok(loaded) => report.loaded = loaded,
            Err(e) => report.warnings.push(format!("load_into_registry: {e}")),
        }
        report
    }

    /// Read every stored `DocType` and register the ones the compiled code did
    /// not declare (`custom = true`).
    ///
    /// Compiled declarations win: a stored row whose name is already registered
    /// is skipped, so a stale or hand-edited row can never shadow code. Returns
    /// the names newly registered, in stored order.
    ///
    /// # Errors
    ///
    /// Returns the engine error when `tabDocType` cannot be read or a row cannot
    /// be decoded into metadata.
    pub async fn load_into_registry(&self) -> Result<Vec<String>> {
        // `DocType` may not be registered yet when this is called (the host may
        // load before it syncs); fall back to a doctype-less context, which the
        // engine still serves through its lazy table path.
        let meta = ferris_schema::get_doctype(DOCTYPE);
        let mut ctx = EngineContext::new(DOCTYPE, "").allow_anonymous(true);
        if let Some(meta) = meta.as_deref() {
            ctx.meta = Some(meta);
        }
        let engine = DocumentEngine::new(self.db.clone());
        let (documents, _) = engine.list(&ctx, &crate::ListQuery::default()).await?;

        let mut loaded = Vec::new();
        for document in documents {
            let stored = Self::meta_of(&document)?;
            if stored.name.is_empty() || ferris_schema::get_doctype(&stored.name).is_some() {
                continue;
            }
            let mut stored = stored;
            stored.custom = true;
            // A persisted fieldtype outside the canonical allowlist is a loud
            // error, not something to silently render as a text box: the rows
            // are written through the validated customization path since
            // , so anything else means the row predates the rule or
            // was tampered with, and both deserve to be seen at boot. Known
            // Unknown names are rejected instead of being stored as-is.
            for field in &mut stored.fields {
                let Some(canonical) = ferris_schema::fieldtype::canonical(&field.fieldtype) else {
                    return Err(Error::Schema(SchemaError::InvalidFieldType(format!(
                        "{}.{}: {}",
                        stored.name, field.fieldname, field.fieldtype
                    ))));
                };
                field.fieldtype = canonical.to_string();
            }
            ferris_schema::register_doctype(stored.clone());
            loaded.push(stored.name);
        }
        Ok(loaded)
    }
}

/// The `DocField` child rows of a DocType, as a JSON array.
fn field_rows(meta: &DoctypeMeta) -> Value {
    let rows: Vec<Value> = meta
        .fields
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let mut row = ferris_core::Map::new();
            row.insert(
                "idx".to_string(),
                Value::from(i64::try_from(i).unwrap_or(i64::MAX) + 1),
            );
            row.insert("parent".to_string(), Value::from(meta.name.clone()));
            row.insert(
                "parenttype".to_string(),
                Value::from(super::DOCTYPE.to_string()),
            );
            row.insert("parentfield".to_string(), Value::from("fields".to_string()));
            row.insert(
                "fieldname".to_string(),
                Value::from(field.fieldname.clone()),
            );
            row.insert(
                "label".to_string(),
                Value::from(field.label.clone().unwrap_or_default()),
            );
            row.insert(
                "fieldtype".to_string(),
                Value::from(field.fieldtype.clone()),
            );
            row.insert(
                "options".to_string(),
                Value::from(field.options.clone().unwrap_or_default()),
            );
            row.insert(
                "default".to_string(),
                Value::from(field.default.clone().unwrap_or_default()),
            );
            row.insert("reqd".to_string(), Value::from(field.reqd));
            row.insert("in_list_view".to_string(), Value::from(field.in_list_view));
            Value::Object(row)
        })
        .collect();
    Value::Array(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_meta() -> DoctypeMeta {
        DoctypeMeta::new("Store Probe", "Build")
            .description("a probe")
            .field(ferris_schema::FieldMeta::data("title").label("Title"))
    }

    #[tokio::test]
    async fn sync_writes_one_row_per_doctype_and_load_is_idempotent() {
        let db = ferris_db::memory_db().await.unwrap();
        let store = DoctypeStore::new(&db);
        let metas = vec![sample_meta(), DoctypeMeta::new("Store Probe 2", "Build")];

        let report = store.sync(&metas).await;
        assert_eq!(report.warnings, Vec::<String>::new(), "{report:?}");
        assert_eq!(report.synced.len(), 2);

        // The rows land in `tabDocType` (the `DocType` table), which is what the
        // reverse direction reads. Before DOC9's fix `upsert` built its context
        // from `meta.name`, so each DocType went to a table of its own and this
        // read was always empty — a vacuous pass. Now the round trip is real: the
        // names are not compiled into this test binary, so loading registers
        // them.
        let mut loaded = store.load_into_registry().await.unwrap();
        loaded.sort();
        assert_eq!(
            loaded,
            vec!["Store Probe".to_string(), "Store Probe 2".to_string()],
            "the synced rows must be readable back from tabDocType"
        );
        // A second load registers nothing new: both are now in the registry.
        let again = store.load_into_registry().await.unwrap();
        assert!(again.is_empty(), "{again:?}");

        // A second sync updates in place instead of colliding on the primary key.
        let report = store.sync(&metas).await;
        assert_eq!(report.warnings, Vec::<String>::new(), "{report:?}");
    }

    #[tokio::test]
    async fn stored_body_survives_a_round_trip() {
        let db = ferris_db::memory_db().await.unwrap();
        let store = DoctypeStore::new(&db);
        let meta = sample_meta();
        let stored = store.upsert(&meta).await.unwrap();
        let back = DoctypeStore::meta_of(&stored).unwrap();
        assert_eq!(back.name, meta.name);
        assert_eq!(back.fields.len(), meta.fields.len());
    }

    /// DOC9: `sync_registry` runs **both** directions — it mirrors compiled
    /// declarations into `tabDocType` **and** reads DB-only DocTypes back into
    /// the registry, so a hand-created `custom` row becomes servable.
    ///
    /// Before the wiring, `load_into_registry` had no caller: the second
    /// direction the module documents (`store.rs:12-16`) never ran, and a row
    /// written by `sync` was never read back.
    #[tokio::test]
    async fn sync_registry_loads_db_only_doctypes_back_into_the_registry() {
        let db = ferris_db::memory_db().await.unwrap();
        let store = DoctypeStore::new(&db);

        // A name no compiled DocType declares, so it can only come from the DB.
        let name = "DbOnlyProbe";
        assert!(
            ferris_schema::get_doctype(name).is_none(),
            "the probe name must not already be registered"
        );
        let meta = DoctypeMeta::new(name, "Custom")
            .field(ferris_schema::FieldMeta::data("title").label("Title"));
        // Write the row the way a hand-created `DocType` row would land, without
        // registering it.
        store.upsert(&meta).await.unwrap();
        assert!(
            ferris_schema::get_doctype(name).is_none(),
            "upsert writes the row but must not register it"
        );

        // `sync_registry` syncs the compiled registry (a no-op for our row) and
        // then loads DB-only rows back — including the probe.
        let report = store.sync_registry().await;
        assert!(
            report.loaded.iter().any(|n| n == name),
            "the DB-only DocType must be reported as loaded: {report:?}"
        );
        let loaded = ferris_schema::get_doctype(name).expect("the probe is now registered");
        assert!(loaded.custom, "a DB-only DocType is registered as custom");
    }
}
