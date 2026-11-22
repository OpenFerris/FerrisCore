//! Applying a `Property Setter` to the effective metadata.
//!
//! One rule per property, kept apart from the store so the wiring (tables,
//! caching, write order) and the semantics (what a property means) can be read
//! and tested separately. Every property here is read by a renderer or by the
//! engine; a name with no reader is a defect, not a placeholder.

use super::{PropertySetter, parse_bool};
use crate::contract::meta::CollectionView;
use crate::registry::{DoctypeMeta, FieldMeta};

/// The deepest first-open expansion the tree accepts. Beyond this a tree with
/// many roots renders its whole document set on the first paint.
pub const MAX_EXPANDED_LEVELS: u8 = 10;

/// Move fields according to their `insert_after` property setters.
///
/// `__first__` is the reserved anchor for "put it at the top". Setters are
/// applied in id order (the store returns them that way), so the last one wins,
/// and a missing anchor leaves the field where it was instead of dropping it.
pub(super) fn apply_field_order(fields: &mut Vec<FieldMeta>, setters: &[PropertySetter]) {
    for setter in setters
        .iter()
        .filter(|setter| setter.property == "insert_after")
    {
        let Some(fieldname) = setter.fieldname.as_deref() else {
            continue;
        };
        if fieldname.is_empty() || fieldname == setter.value {
            continue;
        }
        let Some(index) = fields.iter().position(|field| field.fieldname == fieldname) else {
            continue;
        };
        let first = setter.value == "__first__";
        // A missing anchor must not silently relocate the field to the end: a
        // setter that names a field deleted later is a no-op, not a move.
        if !first && !fields.iter().any(|field| field.fieldname == setter.value) {
            continue;
        }
        let field = fields.remove(index);
        let target = if first {
            0
        } else {
            fields
                .iter()
                .position(|candidate| candidate.fieldname == setter.value)
                .map(|index| index + 1)
                .unwrap_or(fields.len())
        };
        fields.insert(target.min(fields.len()), field);
    }
}

pub(super) fn apply_field_property(field: &mut FieldMeta, property: &str, value: &str) {
    match property {
        "label" => field.label = Some(value.to_string()),
        "fieldtype" => {
            // SCH-02/SEC-13: a `Password`/`Secret` field is hashed or encrypted
            // on write and masked on read. A stored setter that retypes it as a
            // plain field would hand the stored credential to the next reader,
            // so it is not applied. The write path rejects such a setter with a
            // 400; this is the last line of defence for rows that predate it.
            if fieldtype_change_allowed(&field.fieldtype, value) {
                field.fieldtype = value.to_string();
            }
        }
        "options" => field.options = Some(value.to_string()),
        "default" => field.default = Some(value.to_string()),
        "description" => field.description = Some(value.to_string()),
        "placeholder" => field.placeholder = Some(value.to_string()),
        "depends_on" => field.depends_on = Some(value.to_string()),
        "reqd" => field.reqd = parse_bool(value),
        "hidden" => field.hidden = parse_bool(value),
        "read_only" => field.read_only = parse_bool(value),
        "in_list_view" => field.in_list_view = parse_bool(value),
        "in_standard_filter" => field.in_standard_filter = parse_bool(value),
        "unique" => field.unique = parse_bool(value),
        _ => {}
    }
}

pub(super) fn apply_doctype_property(meta: &mut DoctypeMeta, property: &str, value: &str) {
    let trimmed = value.trim();
    match property {
        "title_field" => meta.title_field = Some(value.to_string()),
        "search_fields" => {
            meta.search_fields = value
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        "naming_series" => meta.naming_series = value.to_string(),
        "sort_field" => meta.sort_field = Some(value.to_string()),
        "sort_order" => meta.sort_order = Some(value.to_string()),
        "description" => meta.description = Some(value.to_string()),
        // `DoctypeMeta::date_field` is the field the calendar groups by
        // (`ferris-web`'s `CalendarView` reads it from the metadata). Applying
        // the setter here is what makes it editable from the Desk's DocType
        // settings instead of requiring a code change in the doctype builder;
        // an empty value means "no declared date field" and the calendar falls
        // back to `creation`.
        "date_field" => meta.date_field = none_when_empty(value),
        // ── Collection-view configuration ───────────────────────────────
        // Every one of these is read by exactly one Desk renderer; storing a
        // property nothing consumes is the silent success §4 forbids.
        "default_view" => {
            meta.default_view = CollectionView::from_segment(trimmed);
        }
        "kanban_group_field" => meta.kanban_group_field = none_when_empty(value),
        "kanban_card_fields" => {
            meta.kanban_card_fields = trimmed
                .split(',')
                .map(str::trim)
                .filter(|field| !field.is_empty())
                .map(str::to_string)
                .collect();
        }
        "calendar_end_field" => meta.calendar_end_field = none_when_empty(value),
        "calendar_title_field" => meta.calendar_title_field = none_when_empty(value),
        "tree_parent_field" => meta.tree_parent_field = none_when_empty(value),
        "tree_label_field" => meta.tree_label_field = none_when_empty(value),
        "tree_expanded_levels" => {
            // A malformed number is a cleared property, never a guessed level:
            // the validator refuses it at save time, and a value written by an
            // older client must not silently mean "expand everything".
            if let Ok(levels) = trimmed.parse::<u8>() {
                meta.tree_expanded_levels = Some(levels.min(MAX_EXPANDED_LEVELS));
            }
        }
        "gantt_start_field" => meta.gantt_start_field = none_when_empty(value),
        "gantt_end_field" => meta.gantt_end_field = none_when_empty(value),
        "gantt_dependency_field" => meta.gantt_dependency_field = none_when_empty(value),
        "gantt_progress_field" => meta.gantt_progress_field = none_when_empty(value),
        _ => {}
    }
}

fn none_when_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Fieldtypes whose stored value is a credential: hashed (`Password`) or
/// encrypted (`Secret`). The document engine masks both on read.
const SENSITIVE_FIELDTYPES: [&str; 2] = ["Password", "Secret"];

/// Whether `fieldtype` stores a credential.
fn is_sensitive_fieldtype(fieldtype: &str) -> bool {
    SENSITIVE_FIELDTYPES.contains(&fieldtype.trim())
}

/// Whether a `fieldtype` property setter may change `current` to `next`.
///
/// A sensitive type may become another sensitive type (Password → Secret is a
/// deliberate operator decision) but never a plain one: the field would stop
/// hashing/encrypting on write and masking on read, so the stored credential —
/// an Argon2 hash at best, the ciphertext at worst — would travel on the next
/// read response.
fn fieldtype_change_allowed(current: &str, next: &str) -> bool {
    !is_sensitive_fieldtype(current) || is_sensitive_fieldtype(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(fieldtype: &str) -> FieldMeta {
        FieldMeta {
            fieldname: "api_secret".to_string(),
            fieldtype: fieldtype.to_string(),
            ..Default::default()
        }
    }

    /// SCH-02/SEC-13: a `Password` field keeps its type even if a stored setter
    /// says otherwise.
    #[test]
    fn a_sensitive_fieldtype_is_never_downgraded() {
        let mut meta = field("Password");
        apply_field_property(&mut meta, "fieldtype", "Data");
        assert_eq!(meta.fieldtype, "Password");

        let mut meta = field("Secret");
        apply_field_property(&mut meta, "fieldtype", "Data");
        assert_eq!(meta.fieldtype, "Secret");
    }

    /// Retuning a plain field is still allowed, including making it sensitive.
    #[test]
    fn other_fieldtype_changes_still_apply() {
        let mut meta = field("Data");
        apply_field_property(&mut meta, "fieldtype", "Select");
        assert_eq!(meta.fieldtype, "Select");

        let mut meta = field("Data");
        apply_field_property(&mut meta, "fieldtype", "Password");
        assert_eq!(meta.fieldtype, "Password");

        let mut meta = field("Password");
        apply_field_property(&mut meta, "fieldtype", "Secret");
        assert_eq!(meta.fieldtype, "Secret");
    }
}
