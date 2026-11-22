//! Columns and cell labels derived from doctype metadata.
//!
//! Everything here is a pure function of the metadata the server already sent,
//! so the Desk never hardcodes a doctype name, field list or label.

use crate::meta_types::{DynField, DynMeta};
use serde_json::Value;
use std::collections::HashSet;

/// A display column derived from doctype metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub fieldname: String,
    pub label: String,
    pub width: Option<String>,
    pub sortable: bool,
    pub fieldtype: String,
    pub options: Option<String>,
}

pub fn title_field(meta: &DynMeta) -> &str {
    if let Some(title) = meta.title_field.as_deref()
        && !title.is_empty()
    {
        return title;
    }
    meta.search_fields
        .first()
        .map(|s| s.as_str())
        .unwrap_or("name")
}

/// Timestamps the engine stamps on every document, whether or not the doctype
/// declares them, so they are always worth showing.
///
/// `modified` is deliberately absent: the list table renders it as a fixed
/// column with a relative age, and listing it here would show it twice.
const TIMESTAMP_COLUMNS: &[(&str, &str)] = &[("creation", "Created")];

/// Columns shown in list views.
///
/// A field qualifies when the doctype put it in the list (`in_list_view`), when
/// it is a timestamp, or when a declared index covers it — an index means the
/// app queries the field often enough to have paid for it. Anything else stays
/// out, so the list shows what the app chose rather than every field it has.
///
/// The doctype's own field order comes first, then the columns the doctype does
/// not declare (timestamps, then indexed fields), so the result is stable.
pub fn list_columns(meta: &DynMeta) -> Vec<Column> {
    // Collected once: qualifying per field used to rescan every index, and the
    // extra-column pass rescanned the columns built so far, both per field.
    let indexed: HashSet<&str> = meta
        .indexes
        .iter()
        .flat_map(|i| i.fields.iter().map(String::as_str))
        .collect();
    let declared: HashSet<&str> = meta.fields.iter().map(|f| f.fieldname.as_str()).collect();

    let mut cols: Vec<Column> = meta
        .fields
        .iter()
        .filter(|f| qualifies(&indexed, &f.fieldname, f.in_list_view))
        .map(|f| Column {
            fieldname: f.fieldname.clone(),
            label: label_of(f),
            width: None,
            sortable: true,
            fieldtype: f.fieldtype.clone(),
            options: f.options.clone(),
        })
        .collect();

    let mut seen: HashSet<String> = cols.iter().map(|c| c.fieldname.clone()).collect();
    let mut push_extra = |fieldname: &str, label: &str, fieldtype: &str| {
        if !seen.insert(fieldname.to_string()) {
            return;
        }
        cols.push(Column {
            fieldname: fieldname.to_string(),
            label: label.to_string(),
            width: None,
            sortable: true,
            fieldtype: fieldtype.to_string(),
            options: None,
        });
    };

    for (fieldname, label) in TIMESTAMP_COLUMNS {
        push_extra(fieldname, label, "Datetime");
    }
    // Iterate the declared index order, not the set: the column order is part
    // of the contract with the saved views and the column picker.
    for fieldname in meta.indexes.iter().flat_map(|i| i.fields.iter()) {
        if declared.contains(fieldname.as_str()) {
            continue;
        }
        push_extra(fieldname, fieldname, "Data");
    }

    if !seen.contains("name") {
        cols.insert(
            0,
            Column {
                fieldname: "name".into(),
                label: meta.name.clone(),
                width: None,
                sortable: true,
                fieldtype: "Data".into(),
                options: None,
            },
        );
    }
    cols
}

/// Whether `fieldname` earns a list column, from the three sources that can
/// put a field in a list.
fn qualifies(indexed: &HashSet<&str>, fieldname: &str, in_list_view: bool) -> bool {
    in_list_view
        || TIMESTAMP_COLUMNS.iter().any(|(name, _)| *name == fieldname)
        || indexed.contains(fieldname)
}

/// Fields usable as structural standard filters.
pub fn standard_filter_fields(meta: &DynMeta) -> Vec<Column> {
    meta.fields
        .iter()
        .filter(|f| f.in_standard_filter || matches!(f.fieldtype.as_str(), "Select" | "Check"))
        .map(|f| Column {
            fieldname: f.fieldname.clone(),
            label: label_of(f),
            width: None,
            sortable: false,
            fieldtype: f.fieldtype.clone(),
            options: f.options.clone(),
        })
        .collect()
}

/// Fields the filter builder may target.
///
/// Unlike [`list_columns`], this is not scoped to `in_list_view`: the list's
/// field selection is only the seed for the columns the table opens with, while
/// a filter has to reach any value the document stores. Layout-only breaks carry
/// no value, so they are the only declared fields left out.
///
/// The engine-stamped fields (`name`, `owner`, `docstatus`, `creation`,
/// `modified`) are real columns on every document even when the doctype does not
/// declare them, so they are appended when absent.
pub fn filter_fields(meta: &DynMeta) -> Vec<Column> {
    let mut cols: Vec<Column> = meta
        .fields
        .iter()
        .filter(|f| !ferris_schema::fieldtype::is_layout(&f.fieldtype))
        .map(|f| Column {
            fieldname: f.fieldname.clone(),
            label: label_of(f),
            width: None,
            sortable: false,
            fieldtype: f.fieldtype.clone(),
            options: f.options.clone(),
        })
        .collect();

    let mut seen: HashSet<String> = cols.iter().map(|c| c.fieldname.clone()).collect();

    // `name` is the id every document has; the doctype rarely declares it, and
    // it is the most common filter target, so it goes first.
    if seen.insert("name".to_string()) {
        cols.insert(
            0,
            Column {
                fieldname: "name".into(),
                label: meta.name.clone(),
                width: None,
                sortable: false,
                fieldtype: "Data".into(),
                options: None,
            },
        );
    }

    // Labels are the Desk's; `docstatus` values are the words the document
    // stores (`types.rs` in `ferris-document` explains the column/data split).
    const SYSTEM_FIELDS: &[(&str, &str, &str, Option<&str>)] = &[
        ("owner", "Owner", "Data", None),
        (
            "docstatus",
            "Status",
            "Select",
            Some("draft\nsubmitted\ncancelled"),
        ),
        ("creation", "Created", "Datetime", None),
        ("modified", "Modified", "Datetime", None),
    ];
    for (fieldname, label, fieldtype, options) in SYSTEM_FIELDS {
        if seen.insert((*fieldname).to_string()) {
            cols.push(Column {
                fieldname: (*fieldname).to_string(),
                label: (*label).to_string(),
                width: None,
                sortable: false,
                fieldtype: (*fieldtype).to_string(),
                options: options.map(str::to_string),
            });
        }
    }

    cols
}

/// Fields rendered on the form (visible, non-hidden, includes read-only info).
pub fn form_fields(meta: &DynMeta) -> Vec<&DynField> {
    meta.fields.iter().filter(|f| !f.hidden).collect()
}

/// The label a field renders with: the doctype's own label, or a prettified
/// fieldname.
///
/// It takes no [`DynMeta`]: there is no doctype-level label map to consult, and
/// the parameter that used to sit here was underscored-unused (WS1).
pub fn label_of(f: &DynField) -> String {
    f.label.clone().unwrap_or_else(|| prettify(&f.fieldname))
}

/// Turn a fieldname into a human label when the doctype declares none:
/// `total_amount` -> `Total Amount`, `URLValue` -> `URL Value`.
///
/// A word boundary is a lower→upper transition or an underscore, so a run of
/// capitals stays one word (`URL`, not `U R L`); each word's first letter is
/// capitalised and the rest is left as written.
pub fn prettify(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if !current.is_empty() && c.is_uppercase() {
            let previous = chars[i - 1];
            let next_is_lowercase = chars.get(i + 1).is_some_and(|next| next.is_lowercase());
            let boundary = previous.is_lowercase()
                || previous.is_ascii_digit()
                || (previous.is_uppercase() && next_is_lowercase);
            if boundary {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(c);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a document name is an opaque identifier (a UUID or a long hex
/// string) instead of a human-readable reference.
///
/// A `Link` suggestion reading `Administrator · e025b6f7-d935-…` is noisier than
/// the name alone: the id is the wire value and, when it is needed, the tooltip.
/// Users are the common case (their `name` is a UUID, their `full_name` is the
/// label), but any document named after a generated id benefits.
#[must_use]
pub fn is_opaque_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.len() < 16 {
        return false;
    }
    let hex_only = trimmed
        .chars()
        .all(|ch| ch == '-' || ch.is_ascii_hexdigit());
    hex_only
}

/// The most human label a raw document carries, without its id.
///
/// The same preference order the Link suggestions use: a person's name, then
/// any field that reads as a title. Falls back to the document name.
#[must_use]
pub fn doc_label(doc: &Value) -> String {
    let name = doc.get("name").and_then(Value::as_str).unwrap_or_default();
    let mut best = String::new();
    if let Some(obj) = doc.as_object() {
        for key in [
            // A user (or any document with a human name) must be shown by that
            // name: a share or an activity row reading `e025b6f7-…` is unreadable.
            "full_name",
            "display_name",
            "email",
            "subject",
            "project_name",
            "title",
            "task",
            "short_text",
            "label",
            "description",
            "name",
        ] {
            if let Some(text) = obj.get(key).and_then(Value::as_str)
                && !text.is_empty()
            {
                best = text.to_string();
                break;
            }
        }
    }
    if best.is_empty() {
        name.to_string()
    } else {
        best
    }
}

/// Best-effort display string for a raw document (used by Link suggestions).
///
/// The label, plus the document name when it adds information: a project reads
/// `Alpha · PROJECT00001`, a user reads `Administrator`.
pub fn link_display(doc: &Value) -> String {
    let name = doc.get("name").and_then(Value::as_str).unwrap_or_default();
    let best = doc_label(doc);
    if best.is_empty() || best == name || is_opaque_name(name) {
        best
    } else {
        format!("{} · {}", best, name)
    }
}

/// Record name (id) for a row.
pub fn record_name(row: &Value) -> String {
    row.get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Human-readable cell value for a fieldtype.
pub fn display_value(fieldtype: &str, value: &Value) -> String {
    match (fieldtype, value) {
        (_, Value::Null) => String::new(),
        (_, Value::String(s)) => s.clone(),
        // A boolean cell reads as an answer, not as a Rust literal: the Desk
        // speaks Spanish everywhere else, and `true`/`false` leaked into every
        // list and report that showed a `Check` without the fieldtype.
        (_, Value::Bool(b)) => {
            if *b {
                "Sí".into()
            } else {
                "No".into()
            }
        }
        (_, Value::Number(n)) => n.to_string(),
        (_, Value::Object(_)) | (_, Value::Array(_)) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

/// Normalise docstatus (engine stores "draft"|"submitted"|"cancelled", legacy
/// integer columns used 0/1/2) to the integer rank used by the UI.
pub fn docstatus_rank(value: &Value) -> i64 {
    match value {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => match s.as_str() {
            "submitted" => 1,
            "cancelled" => 2,
            _ => 0,
        },
        _ => 0,
    }
}

pub fn is_submittable(meta: &DynMeta) -> bool {
    meta.is_submittable
}

#[cfg(test)]
mod tests {
    use super::{is_opaque_name, link_display};

    /// A user is labelled by the visual name; the UUID is not appended.
    #[test]
    fn a_link_display_never_appends_an_opaque_id() {
        let doc = serde_json::json!({
            "name": "e025b6f7-d935-4ce9-a10f-27865f94909a",
            "full_name": "Administrator"
        });
        assert_eq!(link_display(&doc), "Administrator");
        let doc = serde_json::json!({
            "name": "PROJECT00001",
            "project_name": "Alpha"
        });
        assert_eq!(link_display(&doc), "Alpha · PROJECT00001");
    }

    #[test]
    fn opaque_names_are_uuids_and_long_hex_ids() {
        assert!(is_opaque_name("e025b6f7-d935-4ce9-a10f-27865f94909a"));
        assert!(is_opaque_name("0123456789abcdef0123456789abcdef"));
        assert!(!is_opaque_name("PROJECT00001"));
        assert!(!is_opaque_name("Alpha"));
        assert!(!is_opaque_name("TASK-0001"));
    }
}
