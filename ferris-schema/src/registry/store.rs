//! The process-global DocType store and its insert path.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use super::standard::add_standard_fields;
use super::types::DoctypeMeta;

pub struct SchemaRegistry {
    pub(super) doctypes: RwLock<HashMap<String, Arc<DoctypeMeta>>>,
    pub(super) by_module: RwLock<HashMap<String, Vec<String>>>,
    /// Who registered each name: the framework/compiled crates, or one app.
    ///
    /// This is the collision policy's source of truth. Comparing `module`
    /// strings would be spoofable — a plugin controls the `module` it declares —
    /// so the app identity is passed separately by the host and recorded here.
    pub(super) origins: RwLock<HashMap<String, Origin>>,
}

/// Provenance of a registered DocType.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Origin {
    /// A compiled crate or the stored-doctype loader: trusted, never
    /// replaceable by an app.
    Framework,
    /// Declared by one app; only that same app may replace it.
    App(String),
}

impl SchemaRegistry {
    pub fn new() -> Self {
        Self {
            doctypes: RwLock::new(HashMap::new()),
            by_module: RwLock::new(HashMap::new()),
            origins: RwLock::new(HashMap::new()),
        }
    }

    /// Register a DocType, replacing any previous metadata for the same name.
    ///
    /// The metadata is first passed through [`add_standard_fields`], so this is
    /// the one choke point that guarantees every registered `DocType` — core,
    /// compiled or plugin-declared — carries the standard fields (currently
    /// `tags`). A declaration that names a standard field itself keeps its own
    /// definition.
    ///
    /// Every field's `fieldtype` is also **canonicalized** through
    /// [`crate::fieldtype::canonical`] here, so the wire spelling (`Small
    /// Text`) is what every consumer sees no matter which producer sent it.
    /// This is a last line of defense, not the strict gate:
    /// `register` cannot return an error (its callers include macro-generated
    /// `OnceLock` initializers, where an error cannot surface without a
    /// panic), so the entry points that accept *untrusted* metadata — the
    /// customization write path, the WASM descriptor registration and the
    /// stored-doctype load path — reject unknown fieldtypes themselves, using
    /// this same list.
    ///
    /// Re-registering is the normal case, not an accident: the `#[doctype]`
    /// macro registers from a `OnceLock` on first access, the core DocTypes are
    /// registered on every boot (`register_core_doctypes`), and a WASM plugin
    /// re-registers on reload. The `doctypes` map replaces by name, so the
    /// module index has to do the same or the *name* leaks: `list_by_module`
    /// appended unconditionally, so a DocType registered twice appeared twice
    /// there while `list` — which reads the map — showed it once. Anything
    /// walking a module's DocTypes would then process it twice.
    pub fn register(&self, mut meta: DoctypeMeta) {
        self.insert_meta(&mut meta, Origin::Framework);
    }

    /// Shared insert path: standard fields, fieldtype canonicalization and the
    /// three indexes (`doctypes`, `by_module`, `origins`) stay in lockstep.
    pub(super) fn insert_meta(&self, meta: &mut DoctypeMeta, origin: Origin) {
        add_standard_fields(meta);
        for field in &mut meta.fields {
            if let Some(canonical) = crate::fieldtype::canonical(&field.fieldtype) {
                field.fieldtype = canonical.to_string();
            }
        }
        let name = meta.name.clone();
        let module = meta.module.clone();

        // Lock order is always `origins` → `doctypes` → `by_module`, the same
        // order the validation path takes, so the two cannot deadlock.
        self.origins
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.clone(), origin);
        let mut doctypes = self.doctypes.write().unwrap_or_else(|e| e.into_inner());
        let previous = doctypes.insert(name.clone(), Arc::new(meta.clone()));

        let mut by_module = self.by_module.write().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = previous {
            // Renamed module: the old index entry would keep the DocType listed
            // under a module it no longer belongs to.
            if previous.module != module {
                if let Some(entries) = by_module.get_mut(&previous.module) {
                    entries.retain(|n| *n != name);
                }
            }
        }
        let names = by_module.entry(module).or_default();
        if !names.contains(&name) {
            names.push(name);
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<DoctypeMeta>> {
        self.doctypes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned()
    }

    pub fn list(&self) -> Vec<String> {
        self.doctypes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// All doctype metadata, stable-ordered by registration order (insertion
    /// order is preserved by the underlying `HashMap` only in practice; sort
    /// by name for a deterministic order).
    pub fn list_metas(&self) -> Vec<Arc<DoctypeMeta>> {
        let mut metas: Vec<_> = self
            .doctypes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        metas.sort_by(|a, b| a.name.cmp(&b.name));
        metas
    }

    pub fn list_by_module(&self, module: &str) -> Vec<String> {
        self.by_module
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(module)
            .cloned()
            .unwrap_or_default()
    }

    pub fn count(&self) -> usize {
        self.doctypes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    pub fn has(&self, name: &str) -> bool {
        self.doctypes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(name)
    }
}

impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) static REGISTRY: LazyLock<SchemaRegistry> = LazyLock::new(SchemaRegistry::new);

pub fn register_doctype(meta: DoctypeMeta) {
    REGISTRY.register(meta);
}

pub fn get_doctype(name: &str) -> Option<Arc<DoctypeMeta>> {
    REGISTRY.get(name)
}

pub fn list_doctypes() -> Vec<String> {
    REGISTRY.list()
}

/// All registered doctype metadata, ordered by name. Deterministic, unlike
/// [`list_doctypes`].
pub fn list_doctypes_metas() -> Vec<Arc<DoctypeMeta>> {
    REGISTRY.list_metas()
}

pub fn list_doctypes_by_module(module: &str) -> Vec<String> {
    REGISTRY.list_by_module(module)
}
