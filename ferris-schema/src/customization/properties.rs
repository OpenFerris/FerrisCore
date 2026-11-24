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
        // ── Formatting and validation ───────────────────────────────────
        "precision" => field.precision = parse_u8(value).map(|n| n.min(10)),
        "max_length" => field.max_length = parse_u32(value),
        "min_value" => field.min_value = none_when_empty(value),
        "max_value" => field.max_value = none_when_empty(value),
        "non_negative" => field.non_negative = parse_bool(value),
        "width" => field.width = none_when_empty(value),
        "print_width" => field.print_width = none_when_empty(value),
        "length" => field.length = parse_u32(value),
        "fetch_from" => field.fetch_from = none_when_empty(value),
        "fetch_if_empty" => field.fetch_if_empty = parse_bool(value),
        // ── Behaviour the Desk and the engine read ─────────────────────
        "bold" => field.bold = parse_bool(value),
        "allow_on_submit" => field.allow_on_submit = parse_bool(value),
        "read_only_on_load" => field.read_only_on_load = parse_bool(value),
        "no_copy" => field.no_copy = parse_bool(value),
        "search_index" => field.search_index = parse_bool(value),
        "sort_options" => field.sort_options = parse_bool(value),
        "in_global_search" => field.in_global_search = parse_bool(value),
        "in_preview" => field.in_preview = parse_bool(value),
        "in_filter" => field.in_filter = parse_bool(value),
        "print_hide" => field.print_hide = parse_bool(value),
        "report_hide" => field.report_hide = parse_bool(value),
        "ignore_user_permissions" => field.ignore_user_permissions = parse_bool(value),
        "ignore_xss_filter" => field.ignore_xss_filter = parse_bool(value),
        "ignore_xss_filter_field" => field.ignore_xss_filter_field = parse_bool(value),
        "convert_to_user_timezone" => field.convert_to_user_timezone = parse_bool(value),
        "remember_last_selected_value" => field.remember_last_selected_value = parse_bool(value),
        "hide_border" => field.hide_border = parse_bool(value),
        "collapsible" => field.collapsible = parse_bool(value),
        "collapsible_depends_on" => field.collapsible_depends_on = none_when_empty(value),
        "mandatory_depends_on" => field.mandatory_depends_on = none_when_empty(value),
        "read_only_depends_on" => field.read_only_depends_on = none_when_empty(value),
        // ── Layout and access ───────────────────────────────────────────
        // `columns` is the span of the field inside its section (1..=4 in the
        // Desk's grid); a value outside the rendered range is clamped rather
        // than stored and ignored.
        "columns" => {
            field.columns = parse_u8(value)
                .map(|n| n.clamp(1, 4))
                .unwrap_or(field.columns)
        }
        // `permlevel` is how the engine hides a field from a caller; `mask`
        // redacts its value. The API additionally requires System Manager to
        // write either one.
        "permlevel" => {
            field.permlevel = parse_u8(value).map(|n| n.min(9)).unwrap_or(field.permlevel)
        }
        "mask" => field.mask = parse_bool(value),
        _ => {}
    }
}

fn parse_u8(value: &str) -> Option<u8> {
    value.trim().parse::<u8>().ok()
}

fn parse_u32(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok()
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

    /// The advanced properties the inspector exposes reach the field, and a
    /// value outside the rendered range is clamped instead of stored and
    /// ignored.
    #[test]
    fn advanced_properties_are_applied_and_bounded() {
        let mut meta = field("Float");
        apply_field_property(&mut meta, "precision", "3");
        apply_field_property(&mut meta, "max_length", "120");
        apply_field_property(&mut meta, "bold", "1");
        apply_field_property(&mut meta, "allow_on_submit", "1");
        apply_field_property(&mut meta, "fetch_from", "customer.tax_id");
        apply_field_property(&mut meta, "columns", "9");
        apply_field_property(&mut meta, "permlevel", "42");
        apply_field_property(
            &mut meta,
            "mandatory_depends_on",
            "eval:doc.status == 'open'",
        );

        assert_eq!(meta.precision, Some(3));
        assert_eq!(meta.max_length, Some(120));
        assert!(meta.bold);
        assert!(meta.allow_on_submit);
        assert_eq!(meta.fetch_from.as_deref(), Some("customer.tax_id"));
        assert_eq!(meta.columns, 4, "the Desk grid has four columns");
        assert_eq!(meta.permlevel, 9, "the engine has ten permission levels");
        assert_eq!(
            meta.mandatory_depends_on.as_deref(),
            Some("eval:doc.status == 'open'")
        );
    }
}
