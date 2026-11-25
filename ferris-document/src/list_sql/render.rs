use super::types::*;
use crate::filters::ListFilter;
use ferris_core::{Result, Value};
use ferris_db::Dialect;
use ferris_db::sql::Ident;
use ferris_schema::FieldMeta;
use ferris_schema::contract::list::FilterOp;

/// How many OR groups `ops` splits into.
///
/// Delegates to [`crate::filters::or_groups`], the one definition of the
/// grouping rule the evaluator (`filters::matches_filters`) also uses — DOC5.
/// A second copy here is what let the planner and the evaluator drift.
pub(crate) fn group_count(ops: &[ListFilter]) -> usize {
    crate::filters::or_groups(ops).len()
}

/// Why `field` cannot be pushed into SQL, or `None` when it can.
pub(crate) fn pushdown_reason(field: &str) -> Option<ResidualReason> {
    if !is_column_field(field) {
        return Some(ResidualReason::NotAColumn);
    }
    if !is_filterable_field(field) {
        return Some(ResidualReason::RepresentedDifferently);
    }
    None
}

/// A `ListFilter` as SQL.
///
/// `Ok(None)` means the operator itself cannot be expressed portably, which is
/// a different thing from the field not being pushable and is reported as such.
pub(crate) fn sql_predicate(
    f: &ListFilter,
) -> std::result::Result<Option<Predicate>, ResidualReason> {
    if let Some(reason) = pushdown_reason(&f.field) {
        return Err(reason);
    }
    // Both filterable columns are `VARCHAR`, and `parse_list_filters` already
    // stringifies its values, so the bound parameter is the text the caller
    // asked about — no coercion, no reinterpretation.
    let value = match &f.value {
        Value::String(s) => Value::string(s.clone()),
        other => Value::string(other.to_json().to_string()),
    };

    let predicate = match f.op {
        FilterOp::Equals => Predicate::Eq {
            column: f.field.clone(),
            value,
        },
        FilterOp::Like => Predicate::Like {
            column: f.field.clone(),
            // `parse_list_filters` hands the needle over as a literal string,
            // so it is bound as a parameter with wildcards added here; no
            // caller-controlled text reaches the SQL.
            needle: format!("%{}%", escape_like(f.value.as_str().unwrap_or_default())),
        },
        FilterOp::In => Predicate::In {
            column: f.field.clone(),
            values: f
                .value
                .as_str()
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(Value::string)
                .collect(),
        },
        FilterOp::Gt => Predicate::Gt {
            column: f.field.clone(),
            value,
        },
        FilterOp::Gte => Predicate::Gte {
            column: f.field.clone(),
            value,
        },
        FilterOp::Lt => Predicate::Lt {
            column: f.field.clone(),
            value,
        },
        FilterOp::Lte => Predicate::Lte {
            column: f.field.clone(),
            value,
        },
    };
    Ok(Some(predicate))
}

/// A `ListFilter` over a declared field's real column, as a SQL predicate.
///
/// `Ok(None)` means the operator has no faithful spelling for this column
/// (for example a range over `Check`); `Err` means the *value* cannot be
/// bound. Both keep the filter in Rust.
pub(crate) fn declared_filter_predicate(
    field: &FieldMeta,
    filter: &ListFilter,
) -> std::result::Result<Option<Predicate>, ResidualReason> {
    let column = field.fieldname.clone();
    let predicate = match filter.op {
        FilterOp::Equals => {
            // `""` also matches a missing cell in Rust (`cell_str`), which is
            // a column `NULL`, not `''`.
            if is_empty_string(&filter.value) {
                return Err(ResidualReason::EmptyStringNotPushed);
            }
            timestamp_guard(field, &filter.value)?;
            let value = coerce_column_value(field, &filter.value)
                .ok_or(ResidualReason::ValueNotCoercible)?;
            Predicate::Eq { column, value }
        }
        FilterOp::In => {
            let parts: Vec<&str> = filter
                .value
                .as_str()
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .collect();
            // An empty part matches a missing cell in Rust, which no SQL `IN`
            // can express.
            if parts.iter().any(|part| part.is_empty()) {
                return Err(ResidualReason::EmptyStringNotPushed);
            }
            let mut values = Vec::with_capacity(parts.len());
            for part in parts {
                values.push(
                    coerce_column_value(field, &Value::string(part))
                        .ok_or(ResidualReason::ValueNotCoercible)?,
                );
            }
            Predicate::In { column, values }
        }
        FilterOp::Like => {
            if !is_text_column(field) {
                return Ok(None);
            }
            let needle = filter.value.as_str().unwrap_or_default();
            // An empty substring matches every cell in Rust, including a
            // missing one; `LIKE '%%'` never matches SQL `NULL`.
            if needle.is_empty() {
                return Err(ResidualReason::EmptyStringNotPushed);
            }
            Predicate::Like {
                column,
                needle: format!("%{}%", escape_like(needle)),
            }
        }
        FilterOp::Gt | FilterOp::Gte | FilterOp::Lt | FilterOp::Lte => {
            // Booleans have no meaningful order in the evaluator: it compares
            // their `true`/`false` text. Keep that in Rust.
            if !is_text_column(field) && !is_numeric_column(field) {
                return Ok(None);
            }
            if is_text_column(field) {
                timestamp_guard(field, &filter.value)?;
            }
            let value = coerce_column_value(field, &filter.value)
                .ok_or(ResidualReason::ValueNotCoercible)?;
            match filter.op {
                FilterOp::Gt => Predicate::Gt { column, value },
                FilterOp::Gte => Predicate::Gte { column, value },
                FilterOp::Lt => Predicate::Lt { column, value },
                FilterOp::Lte => Predicate::Lte { column, value },
                _ => unreachable!("matched above"),
            }
        }
    };
    Ok(Some(predicate))
}

/// Neutralise `LIKE` wildcards in a bound needle.
///
/// The needle is already a *parameter*, so this is not an injection defence —
/// it is a correctness one. A search for `100%` must find `100%`, not match
/// every row starting with `100`.
pub(crate) fn escape_like(needle: &str) -> String {
    needle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(crate) fn render_predicate(
    predicate: &Predicate,
    dialect: Dialect,
    params: &mut Vec<Value>,
) -> Result<String> {
    match predicate {
        Predicate::Eq { column, value } => {
            let column = Ident::new(column)?.quote(dialect);
            params.push(value.clone());
            Ok(format!("{column} = ?"))
        }
        Predicate::Like { column, needle } => {
            let column = Ident::new(column)?.quote(dialect);
            params.push(Value::string(needle.clone()));
            if dialect.supports_ilike() {
                Ok(format!("{column} ILIKE ?"))
            } else {
                Ok(format!("LOWER({column}) LIKE LOWER(?)"))
            }
        }
        Predicate::In { column, values } => {
            if values.is_empty() {
                return Ok("1 = 0".to_string());
            }
            let column = Ident::new(column)?.quote(dialect);
            params.extend(values.iter().cloned());
            let holes = vec!["?"; values.len()].join(", ");
            Ok(format!("{column} IN ({holes})"))
        }
        Predicate::Gt { column, value } => {
            let column = Ident::new(column)?.quote(dialect);
            params.push(value.clone());
            Ok(format!("{column} > ?"))
        }
        Predicate::Gte { column, value } => {
            let column = Ident::new(column)?.quote(dialect);
            params.push(value.clone());
            Ok(format!("{column} >= ?"))
        }
        Predicate::Lt { column, value } => {
            let column = Ident::new(column)?.quote(dialect);
            params.push(value.clone());
            Ok(format!("{column} < ?"))
        }
        Predicate::Lte { column, value } => {
            let column = Ident::new(column)?.quote(dialect);
            params.push(value.clone());
            Ok(format!("{column} <= ?"))
        }
    }
}
