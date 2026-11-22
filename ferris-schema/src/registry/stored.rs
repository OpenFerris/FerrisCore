//! Stored `DocType` metadata: Actions, Links and Permission Types.
//!
//! Frappe keeps these three as ordinary `DocType` documents (`DocType Action`,
//! `DocType Link`, `Permission Type`) and folds them into the metadata payload.
//! The rows are site data, not compiled metadata, so they are read here — in
//! `ferris-schema`, the crate every consumer already has — instead of in one
//! API layer. The REST endpoint, the SSR bridge and the WASM `data` import then
//! project the same bytes, and a permission row cannot disagree with the
//! payload a Desk renders.
//!
//! Missing tables mean "no rows", never an error: a database that never created
//! its metadata `DocType` documents is a valid empty install. A malformed row is corrupt
//! metadata and fails loudly.

// `ferris_core::Error` is the shared taxonomy; boxing it would make every
// caller match on a Box it never inspects.
#![allow(clippy::result_large_err)]

use ferris_core::{Error, Result, SerializationError, ValidationError, Value};
use ferris_db::sql::Ident;
use ferris_db::{Db, Stmt};

use crate::contract::meta::{DynCustomPermission, DynDocAction, DynDocLink, DynMeta};

use super::DoctypeMeta;
use crate::CustomizationStore;

const ACTION_TABLE: &str = "tabDocTypeAction";
const LINK_TABLE: &str = "tabDocTypeLink";
const PERMISSION_TYPE_TABLE: &str = "tabPermissionType";

/// Every stored row of `table` whose `document_type` is `doctype`.
async fn rows_for(pool: &Db, table: &str, doctype: &str) -> Result<Vec<Value>> {
    if !pool.table_exists(table).await? {
        return Ok(Vec::new());
    }
    let quoted = Ident::new(table)?.quote(pool.dialect());
    let rows = pool
        .fetch_all(&Stmt::new(
            format!("SELECT data FROM {quoted} ORDER BY name"),
            Vec::new(),
        ))
        .await?;
    let mut documents = Vec::with_capacity(rows.len());
    for row in rows {
        let data = row.str("data")?;
        let value: Value = serde_json::from_str(&data)
            .map_err(|error| Error::Serialization(SerializationError::Json(error)))?;
        if value.get("document_type").and_then(Value::as_str) == Some(doctype) {
            documents.push(value);
        }
    }
    Ok(documents)
}

fn text(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn optional_text(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn toggle(value: &Value, field: &str) -> bool {
    value.get(field).is_some_and(|value| match value {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_i64().is_some_and(|n| n != 0),
        Value::String(text) => text == "1" || text.eq_ignore_ascii_case("true"),
        Value::Null | Value::Array(_) | Value::Object(_) => false,
    })
}

/// The actions declared for `doctype`, in stable display order.
///
/// # Errors
///
/// A malformed stored row, or a database failure while reading the table.
pub async fn doc_actions(pool: &Db, doctype: &str) -> Result<Vec<DynDocAction>> {
    let mut actions: Vec<DynDocAction> = rows_for(pool, ACTION_TABLE, doctype)
        .await?
        .into_iter()
        .map(|row| DynDocAction {
            name: text(&row, "name"),
            label: text(&row, "label"),
            action_type: text(&row, "action_type"),
            action: text(&row, "action"),
            group: optional_text(&row, "group"),
            hidden: toggle(&row, "hidden"),
        })
        .collect();
    actions.sort_by(|left, right| {
        left.group
            .cmp(&right.group)
            .then_with(|| left.label.cmp(&right.label))
    });
    Ok(actions)
}

/// The connections declared for `doctype`, in stable display order.
///
/// # Errors
///
/// A malformed stored row, or a database failure while reading the table.
pub async fn doc_links(pool: &Db, doctype: &str) -> Result<Vec<DynDocLink>> {
    let mut links: Vec<DynDocLink> = rows_for(pool, LINK_TABLE, doctype)
        .await?
        .into_iter()
        .map(|row| DynDocLink {
            name: text(&row, "name"),
            link_doctype: text(&row, "link_doctype"),
            link_fieldname: text(&row, "link_fieldname"),
            table_fieldname: optional_text(&row, "table_fieldname"),
            group: optional_text(&row, "group"),
            hidden: toggle(&row, "hidden"),
        })
        .collect();
    links.sort_by(|left, right| {
        left.group
            .cmp(&right.group)
            .then_with(|| left.link_doctype.cmp(&right.link_doctype))
    });
    Ok(links)
}

/// The permission types declared for `doctype`.
///
/// `role` is empty: this is the *declaration*. The assignment to roles lives in
/// `Custom DocPerm.custom_permissions` and is reported by the permission
/// inventory.
///
/// # Errors
///
/// A malformed stored row, or a database failure while reading the table.
pub async fn permission_types(pool: &Db, doctype: &str) -> Result<Vec<DynCustomPermission>> {
    let mut permissions: Vec<DynCustomPermission> = rows_for(pool, PERMISSION_TYPE_TABLE, doctype)
        .await?
        .into_iter()
        .map(|row| DynCustomPermission {
            perm_type: text(&row, "perm_type"),
            role: String::new(),
        })
        .filter(|permission| !permission.perm_type.is_empty())
        .collect();
    permissions.sort_by(|left, right| left.perm_type.cmp(&right.perm_type));
    permissions.dedup_by(|left, right| left.perm_type == right.perm_type);
    Ok(permissions)
}

/// Effective metadata for `doctype`: the registry definition with the site's
/// custom fields, property setters and `Custom DocPerm` overrides applied.
///
/// This is the same merge the REST endpoint and the SSR bridge use. A caller
/// that only needs the compiled shape uses [`super::get_doctype`]; a caller
/// that renders a form or answers a permission question uses this.
///
/// # Errors
///
/// A database failure while loading customizations or overrides, or a malformed
/// stored customization.
pub async fn effective_meta(pool: &Db, doctype: &str) -> Result<Option<DoctypeMeta>> {
    let Some(meta) = super::get_doctype(doctype) else {
        return Ok(None);
    };
    if let Some(cached) = crate::customization::cached_meta(pool.id(), doctype) {
        return Ok(Some(cached));
    }
    let generation = crate::customization::meta_generation(doctype);
    // The DDL ran at boot/migration. This fallback covers a process that never
    // booted the server (a test, an embedder) and is guarded per `Db::id`, so a
    // request no longer pays for `ensure_table`/`ensure_index` on every call.
    crate::customization::ensure_tables(pool).await?;
    let store = CustomizationStore::with_db(pool.clone());
    let mut meta = store.apply(&meta).await?;
    crate::custom_docperm::apply(pool, doctype, &mut meta).await?;
    crate::customization::cache_meta(pool.id(), doctype, generation, &meta);
    Ok(Some(meta))
}

/// Build the complete wire metadata for `meta`: the registry view plus the
/// stored Actions, Links and Permission Types.
///
/// This is the one projection the REST endpoint, the SSR bridge and the WASM
/// `data` import use. A caller that needs the *effective* metadata (with site
/// customizations applied) passes the already-effective `meta`.
///
/// # Errors
///
/// A stored row that cannot be decoded, or a metadata shape that cannot be
/// projected onto the wire contract.
pub async fn dyn_meta(pool: &Db, meta: &DoctypeMeta) -> Result<DynMeta> {
    let mut payload = DynMeta::from_serde_meta(meta)
        .map_err(|error| Error::Validation(ValidationError::Invalid(error)))?;
    payload.actions = doc_actions(pool, &meta.name).await?;
    payload.links = doc_links(pool, &meta.name).await?;
    payload.custom_permissions = permission_types(pool, &meta.name).await?;
    Ok(payload)
}

/// Serialize [`dyn_meta`] as the JSON string the WIT `data` import carries.
///
/// # Errors
///
/// [`dyn_meta`]'s failures, plus a serialization failure.
pub async fn dyn_meta_json(pool: &Db, meta: &DoctypeMeta) -> Result<String> {
    let payload = dyn_meta(pool, meta).await?;
    serde_json::to_string(&payload)
        .map_err(|error| Error::Serialization(SerializationError::Json(error)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(value: serde_json::Value) -> Value {
        serde_json::from_value(value).expect("the fixture is a Value")
    }

    #[test]
    fn a_toggle_accepts_the_three_wire_spellings_and_rejects_the_rest() {
        for raw in [
            serde_json::json!({"flag": true}),
            serde_json::json!({"flag": 1}),
            serde_json::json!({"flag": "1"}),
            serde_json::json!({"flag": "true"}),
        ] {
            assert!(toggle(&document(raw), "flag"));
        }
        for raw in [
            serde_json::json!({"flag": false}),
            serde_json::json!({"flag": 0}),
            serde_json::json!({"flag": "0"}),
            serde_json::json!({"flag": null}),
            serde_json::json!({}),
        ] {
            assert!(!toggle(&document(raw), "flag"));
        }
    }
}
