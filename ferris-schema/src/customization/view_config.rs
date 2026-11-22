//! Validation of the doctype-level collection-view settings.
//!
//! A setter that names a field the doctype does not declare — or one whose
//! fieldtype cannot carry the property — would be stored and then ignored by
//! the renderer: an empty view with no explanation. `set_property` calls this
//! at the write boundary with the **effective** metadata (custom fields
//! included), so a field added at runtime is a valid target and a typo is
//! refused before it becomes a dead row.

use ferris_core::{Error, Result, SchemaError, ValidationError};

use super::properties::MAX_EXPANDED_LEVELS;
use crate::contract::meta::CollectionView;
use crate::registry::{DoctypeMeta, FieldMeta};

/// The field a doctype-level view property must name, and the types that make
/// the property meaningful.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldRule {
    /// A multi-value `Select` or a `Link`: the board needs stable columns.
    Grouping,
    /// A `Date`/`Datetime` field.
    Date,
    /// A `Link` that points at the doctype being configured.
    SelfLink,
    /// A numeric field 0–100.
    Progress,
    /// Any field the doctype declares (a title/label).
    Any,
}

/// The rule for a view property that names a field, or `None` when the property
/// is not a field reference.
fn field_rule(property: &str) -> Option<FieldRule> {
    match property {
        "date_field" | "calendar_end_field" | "gantt_start_field" | "gantt_end_field" => {
            Some(FieldRule::Date)
        }
        "kanban_group_field" => Some(FieldRule::Grouping),
        "tree_parent_field" | "gantt_dependency_field" => Some(FieldRule::SelfLink),
        "gantt_progress_field" => Some(FieldRule::Progress),
        "calendar_title_field" | "tree_label_field" => Some(FieldRule::Any),
        _ => None,
    }
}

/// Refuse a doctype-level setter the renderer could not honour.
///
/// `Result` is the crate's one error type and its `Err` variant is large; this
/// helper is on the write path of a settings dialog, not a hot loop, so the
/// boxed variant would only add an allocation for no gain.
#[allow(clippy::result_large_err)]
///
/// Called by [`CustomizationStore::set_property`] with the **effective**
/// metadata (custom fields included), so a field added at runtime is a valid
/// target. The error names the property, the field and what was expected; it is
/// a [`ValidationError::InvalidValue`] because the failing input is a setting,
/// not a document field.
pub(super) fn validate_doctype_view_property(
    doctype: &str,
    meta: &DoctypeMeta,
    property: &str,
    value: &str,
) -> Result<()> {
    let invalid = |field: &str, message: String| {
        Err(Error::Validation(ValidationError::InvalidValue {
            field: field.to_string(),
            message,
        }))
    };
    let value = value.trim();
    if value.is_empty() {
        // Clearing a setting is always valid; `apply` restores the declared
        // value.
        return Ok(());
    }

    match property {
        "default_view" => {
            let Some(view) = CollectionView::from_segment(value) else {
                return invalid(
                    property,
                    format!(
                        "«{value}» no es una vista: usa {}",
                        CollectionView::ALL
                            .iter()
                            .map(|view| view.segment())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            };
            if !view_enabled(meta, view) {
                return invalid(
                    property,
                    format!(
                        "la vista «{}» no está habilitada en este DocType (activa su marca in_* primero)",
                        view.segment()
                    ),
                );
            }
            Ok(())
        }
        "kanban_card_fields" => {
            for field in value.split(',').map(str::trim).filter(|f| !f.is_empty()) {
                require_field(meta, doctype, property, field, FieldRule::Any)?;
            }
            Ok(())
        }
        "tree_expanded_levels" => match value.parse::<u8>() {
            Ok(levels) if levels <= MAX_EXPANDED_LEVELS => Ok(()),
            _ => invalid(
                property,
                format!("usa un número entre 0 y {MAX_EXPANDED_LEVELS}"),
            ),
        },
        _ => {
            let Some(rule) = field_rule(property) else {
                return Ok(());
            };
            let field = require_field(meta, doctype, property, value, rule)?;
            // A grouping field must offer at least two columns; a `Select`
            // with one option is a label, and the board would have a single
            // column with no move.
            if rule == FieldRule::Grouping
                && field.fieldtype == "Select"
                && field
                    .options
                    .as_deref()
                    .unwrap_or_default()
                    .lines()
                    .filter(|option| !option.trim().is_empty())
                    .count()
                    < 2
            {
                return invalid(
                    property,
                    format!("«{value}» es un Select con menos de dos opciones"),
                );
            }
            Ok(())
        }
    }
}

/// Resolve `field` and check it against `rule`, returning it for extra checks.
#[allow(clippy::result_large_err)] // same one error type as every other fallible fn here
fn require_field<'a>(
    meta: &'a DoctypeMeta,
    doctype: &str,
    property: &str,
    field: &str,
    rule: FieldRule,
) -> Result<&'a FieldMeta> {
    let Some(found) = meta.fields.iter().find(|f| f.fieldname == field) else {
        return Err(Error::Schema(SchemaError::FieldNotFound {
            doctype: doctype.to_string(),
            field: field.to_string(),
        }));
    };
    let expected = match rule {
        FieldRule::Grouping => "un Select con varias opciones o un Link",
        FieldRule::Date => "un campo Date o Datetime",
        FieldRule::SelfLink => "un Link a este mismo DocType",
        FieldRule::Progress => "un campo Percent, Int o Float",
        FieldRule::Any => "",
    };
    let type_ok = match rule {
        FieldRule::Grouping => match found.fieldtype.as_str() {
            "Link" => true,
            "Select" => found.options.as_deref().unwrap_or_default().contains('\n'),
            _ => false,
        },
        FieldRule::Date => matches!(found.fieldtype.as_str(), "Date" | "Datetime"),
        FieldRule::SelfLink => {
            found.fieldtype == "Link" && found.options.as_deref().map(str::trim) == Some(doctype)
        }
        FieldRule::Progress => matches!(found.fieldtype.as_str(), "Percent" | "Int" | "Float"),
        FieldRule::Any => true,
    };
    if !type_ok {
        return Err(Error::Validation(ValidationError::InvalidValue {
            field: property.to_string(),
            message: format!("«{field}» debe ser {expected}"),
        }));
    }
    Ok(found)
}

/// Whether the doctype's definition enables `view`.
const fn view_enabled(meta: &DoctypeMeta, view: CollectionView) -> bool {
    match view {
        // The list is the index every doctype has.
        CollectionView::List => true,
        CollectionView::Kanban => meta.in_kanban,
        CollectionView::Calendar => meta.in_calendar,
        CollectionView::Tree => meta.in_tree_view,
        CollectionView::Gantt => meta.in_gantt,
        CollectionView::Report => meta.in_report_view,
    }
}
