use super::types::*;
use crate::filters::ListFilter;
use ferris_core::{Result, Value};
use ferris_db::Dialect;
use ferris_db::sql::{Ident, json_field_target};
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
    json_indexed_fields: &[String],
) -> std::result::Result<Option<Predicate>, ResidualReason> {
    let is_json_indexed = json_indexed_fields.iter().any(|x| x == &f.field);
    if is_json_indexed {
        // The Rust `single_matches` uses the same string coercion for the
        // needle/cell; for an exact match on a string field that lives in JSON
        // the unquoted text equality matches it. Ranges are deliberately left
        // in Rust because `filters::cmp` is numeric-when-both-parse and
        // case-insensitive, a set of rules SQL text comparison does not satisfy.
        let value = match &f.value {
            Value::String(s) => Value::string(s.clone()),
            other => Value::string(other.to_json().to_string()),
        };
        match f.op {
            FilterOp::Equals => {
                // `""` is exactly what `cell_str` also yields for a missing
                // cell, so the Rust `eq ""` matches rows where the field is
                // absent or empty while `json_extract(...) = ''` never matches
                // a SQL NULL. Pushing it would drop the rows the Rust filter
                // keeps.
                //
                // A timestamp is the same kind of trap: the Rust evaluator now
                // compares it at the precision the filter names (`equals 12:34`
                // matches `12:34:56`), while the exact-text index only matches
                // the literal `12:34`. Keep it in Rust so both paths agree.
                if value
                    .as_str()
                    .is_some_and(|s| s.is_empty() || crate::filters::is_timestamp_value(s))
                {
                    return Err(ResidualReason::JsonExactNotPushed);
                }
                return Ok(Some(Predicate::JsonEq {
                    field: f.field.clone(),
                    value,
                }));
            }
            FilterOp::In => {
                let values: Vec<String> = f
                    .value
                    .as_str()
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                // An empty part compares equal to a missing cell in Rust
                // (`cell_str` reads it as `""`), so `in a,` matches rows
                // without the field. SQL `IN` cannot express that, so any empty
                // part keeps the whole filter in Rust.
                if values.iter().any(String::is_empty) {
                    return Err(ResidualReason::JsonExactNotPushed);
                }
                return Ok(Some(Predicate::JsonIn {
                    field: f.field.clone(),
                    values,
                }));
            }
            FilterOp::Like => return Err(ResidualReason::JsonLikeNotPushed),
            FilterOp::Gt | FilterOp::Gte | FilterOp::Lt | FilterOp::Lte => {
                return Err(ResidualReason::JsonRangeNotPushed);
            }
        }
    }
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
        Predicate::JsonEq { field, value } => {
            let target = json_field_target(dialect, field)?;
            params.push(value.clone());
            Ok(format!("{target} = ?"))
        }
        Predicate::JsonIn { field, values } => {
            if values.is_empty() {
                return Ok("1 = 0".to_string());
            }
            let target = json_field_target(dialect, field)?;
            params.extend(values.iter().map(|v| Value::string(v.clone())));
            let holes = vec!["?"; values.len()].join(", ");
            Ok(format!("{target} IN ({holes})"))
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
            params.extend(values.iter().map(|v| Value::string(v.clone())));
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
