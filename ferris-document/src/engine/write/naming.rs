use super::super::support::*;
use super::super::*;
use ferris_core::{DocumentError, Error, Result, ValidationError, Value};
use ferris_db::Stmt;

/// The optional metadata table Frappe consults before every other naming rule.
/// The physical table of Frappe's `Document Naming Rule` DocType.
///
/// Engine tables drop the whitespace of the DocType name (`tabToDo`,
/// `tabDocumentNamingRule`); using the display name here would fail the
/// identifier gate and make every insert look like "no rules".
fn rule_table() -> Result<String> {
    table_name("Document Naming Rule")
}

/// Mint the `name` of a new document.
///
/// Priority follows Frappe
/// (<https://docs.frappe.io/framework/user/en/basics/doctypes/naming>):
/// a matching **Document Naming Rule** wins, then the DocType's declared
/// [`NamingType`]. A Child DocType never follows naming rules: it is addressed
/// by its parent row, so a global counter would leak names across parents.
pub(crate) async fn mint_name(
    ctx: &EngineContext<'_>,
    fields: &HashMap<String, Value>,
    naming: &mut WriteSink<'_>,
) -> Result<String> {
    if !ctx.meta.is_some_and(|meta| meta.is_child_table)
        && let Some(name) = document_naming_rule(ctx, fields, naming).await?
    {
        return Ok(name);
    }

    match ctx.naming_type {
        NamingType::Series => {
            if ctx.naming_series.is_empty() {
                // A `Series` declaration with no series has nothing to format;
                // fall back to an opaque name rather than erroring on a doctype
                // whose declaration is merely incomplete.
                Ok(uuid::Uuid::new_v4().to_string())
            } else {
                crate::naming::NamingSeriesStore::next_via(naming, &ctx.naming_series).await
            }
        }
        NamingType::Uuid => Ok(uuid::Uuid::new_v4().to_string()),
        NamingType::AutoIncrement => {
            // A dedicated key keeps the auto-increment counter independent of
            // any naming series the same doctype might also use.
            let key = format!("autoincrement:{}", ctx.doctype);
            crate::naming::NamingSeriesStore::next_auto_via(naming, &key).await
        }
        NamingType::Field => {
            let field = ctx.naming_field.as_deref().ok_or_else(|| {
                Error::Document(DocumentError::NamingFieldUndeclared {
                    doctype: ctx.doctype.clone(),
                })
            })?;
            let value = field_str(fields, field).unwrap_or_default();
            if value.is_empty() {
                return Err(Error::Document(DocumentError::NamingFieldEmpty {
                    doctype: ctx.doctype.clone(),
                    field: field.to_string(),
                }));
            }
            Ok(value)
        }
        NamingType::Prompt => Err(Error::Document(DocumentError::NameRequired {
            doctype: ctx.doctype.clone(),
        })),
        NamingType::NamingSeries => {
            // Frappe reads the pattern from the document's own
            // `naming_series` field, so two documents of the same DocType may
            // follow different series (e.g. `SINV-` and `CINV-`).
            let pattern = field_str(fields, "naming_series")
                .filter(|pattern| !pattern.is_empty())
                .unwrap_or_else(|| ctx.naming_series.clone());
            if pattern.is_empty() {
                return Err(Error::Document(DocumentError::NamingSeriesMissing {
                    doctype: ctx.doctype.clone(),
                }));
            }
            crate::naming::NamingSeriesStore::next_via(naming, &pattern).await
        }
        NamingType::Expression => {
            let pattern = expression_pattern(ctx, fields)?;
            let key = format!("expression:{}:{}", ctx.doctype, pattern);
            let counter = next_counter(naming, &key).await?;
            Ok(crate::naming::format_expression(&pattern, fields, counter))
        }
        NamingType::ExpressionOldStyle => {
            let pattern = expression_pattern(ctx, fields)?;
            let key = format!("expression_old_style:{}:{}", ctx.doctype, pattern);
            let counter = next_counter(naming, &key).await?;
            Ok(crate::naming::format_expression_old_style(
                &pattern, fields, counter,
            ))
        }
        NamingType::Random => Ok(crate::naming::random_name()),
        NamingType::Script => {
            let provider = crate::naming::naming_provider(&ctx.doctype).ok_or_else(|| {
                Error::Document(DocumentError::NamingScriptUndeclared {
                    doctype: ctx.doctype.clone(),
                })
            })?;
            let document = Value::Object(fields.clone());
            let name = provider(&document, ctx)?;
            if name.is_empty() {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "the naming provider for `{}` returned an empty name",
                    ctx.doctype
                ))));
            }
            Ok(name)
        }
    }
}

/// The pattern of an Expression naming method.
///
/// An Expression is declared either on the DocType (`naming_series`) or, like
/// Frappe's controller `autoname`, per document through `naming_series`. The
/// document value wins so a DocType can keep several patterns alive.
fn expression_pattern(ctx: &EngineContext<'_>, fields: &HashMap<String, Value>) -> Result<String> {
    let pattern = field_str(fields, "naming_series")
        .filter(|pattern| !pattern.is_empty())
        .unwrap_or_else(|| ctx.naming_series.clone());
    if pattern.is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "`{}` declares an expression naming type but no `naming_series` pattern",
            ctx.doctype
        ))));
    }
    Ok(pattern)
}

/// Bump the counter for an expression, returning the number (not its padding).
async fn next_counter(naming: &mut WriteSink<'_>, key: &str) -> Result<u64> {
    let counter = crate::naming::NamingSeriesStore::next_auto_via(naming, key).await?;
    counter.parse::<u64>().map_err(|error| {
        Error::Validation(ValidationError::Invalid(format!(
            "naming counter `{key}` is not a number: {error}"
        )))
    })
}

/// The columns a `Document Naming Rule` row is read from.
///
/// Columns are the only source of truth (ADR 0001); the rule stores its
/// counter, prefix and match condition as columns. A table created before this
/// layout lacks them, which is a schema mismatch the SELECT reports.
const RULE_COLUMNS: [&str; 10] = [
    "document_type",
    "priority",
    "prefix",
    "prefix_digits",
    "digits",
    "counter",
    "apply_if",
    "fieldname",
    "condition",
    "disabled",
];

/// Apply the highest-priority matching Document Naming Rule, if one exists.
///
/// The rule table is optional metadata: a database that never migrated its
/// DocType catalogue has no rules, which is not the same as a broken insert, so
/// a missing table is treated as "no rules". Any other read failure propagates.
async fn document_naming_rule(
    ctx: &EngineContext<'_>,
    fields: &HashMap<String, Value>,
    naming: &mut WriteSink<'_>,
) -> Result<Option<String>> {
    if !naming.table_exists(&rule_table()?).await? {
        return Ok(None);
    }
    let dialect = naming.dialect();
    let quoted = q(dialect, &rule_table()?)?;
    let select = RULE_COLUMNS
        .iter()
        .map(|column| q(dialect, column))
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    // Only this DocType's rules can apply: the table holds every DocType's, and
    // reading them all on each insert was pure waste.
    let rows = naming
        .fetch_all(&Stmt::new(
            format!(
                "SELECT {name}, {select} FROM {quoted} WHERE document_type = ? ORDER BY name",
                name = q(dialect, "name")?
            ),
            vec![Value::from(ctx.doctype.as_str())],
        ))
        .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    // One query for every rule's conditions instead of one per rule: the child
    // table is tiny and its rows are only meaningful for the parents here.
    let mut conditions = naming_rule_conditions(naming).await?;

    let mut rules: Vec<(i64, String, Value)> = Vec::new();
    for row in rows {
        let name = row.str("name")?;
        if row.opt_bool("disabled").unwrap_or(false) {
            continue;
        }
        let priority = row.opt_i64("priority").unwrap_or(0);
        let mut rule = ferris_core::Map::new();
        for column in RULE_COLUMNS {
            if let Some(value) = row.opt_str(column) {
                rule.insert(column.to_string(), Value::from(value));
            } else if let Some(value) = row.opt_i64(column) {
                rule.insert(column.to_string(), Value::from(value));
            }
        }
        if let Some(children) = conditions.remove(&name) {
            rule.insert("conditions".to_string(), Value::Array(children));
        }
        rules.push((priority, name, Value::Object(rule)));
    }
    // Higher priority first; the name is the tie-breaker so two rules of the
    // same priority cannot apply in a different order between processes.
    rules.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    for (_, row_name, rule) in rules {
        if !rule_matches(&rule, fields)? {
            continue;
        }
        return mint_from_rule(&rule, &row_name, naming).await.map(Some);
    }
    Ok(None)
}

/// The stored `Document Naming Rule Condition` rows, grouped by their rule.
///
/// One query for the whole child table: the caller then picks the conditions of
/// the rule it is evaluating. A site without the child table simply has no
/// conditions.
async fn naming_rule_conditions(naming: &mut WriteSink<'_>) -> Result<HashMap<String, Vec<Value>>> {
    let mut grouped: HashMap<String, Vec<Value>> = HashMap::new();
    let table = table_name("Document Naming Rule Condition")?;
    if !naming.table_exists(&table).await? {
        return Ok(grouped);
    }
    let dialect = naming.dialect();
    let quoted = q(dialect, &table)?;
    let rows = naming
        .fetch_all(&Stmt::new(
            format!(
                "SELECT parent, field, condition, value FROM {quoted} \
                 WHERE parentfield = ? ORDER BY parent, idx"
            ),
            vec![Value::from("conditions")],
        ))
        .await?;
    for row in &rows {
        let Some(parent) = row.opt_str("parent") else {
            continue;
        };
        let mut condition = ferris_core::Map::new();
        for column in ["field", "condition", "value"] {
            if let Some(value) = row.opt_str(column) {
                condition.insert(column.to_string(), Value::from(value));
            }
        }
        grouped
            .entry(parent)
            .or_default()
            .push(Value::Object(condition));
    }
    Ok(grouped)
}

/// Whether a Document Naming Rule applies to this document.
///
/// Frappe's current doctype stores conditions in a child table
/// (`Document Naming Rule Condition`: field / condition / value, all ANDed);
/// the older form stored a single `apply_if` + `fieldname` or `condition`. Both
/// are accepted so an upgraded site keeps naming.
fn rule_matches(rule: &Value, fields: &HashMap<String, Value>) -> Result<bool> {
    if let Some(conditions) = rule.get("conditions").and_then(Value::as_array)
        && !conditions.is_empty()
    {
        for condition in conditions {
            let field = condition.get("field").and_then(Value::as_str).unwrap_or("");
            let operator = condition
                .get("condition")
                .and_then(Value::as_str)
                .unwrap_or("");
            let expected = condition.get("value").and_then(Value::as_str).unwrap_or("");
            if !compare(fields.get(field), operator, expected)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }

    let apply_if = rule
        .get("apply_if")
        .and_then(Value::as_str)
        .unwrap_or("Fieldname");
    if apply_if == "Condition" {
        let condition = rule
            .get("condition")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if condition.is_empty() {
            return Ok(false);
        }
        return evaluate_condition(condition, fields);
    }
    // "Fieldname" (and the older implicit form): the named field must be set.
    // Without a fieldname there is nothing to test, so the rule cannot apply.
    let field = rule.get("fieldname").and_then(Value::as_str).unwrap_or("");
    Ok(!field.is_empty() && truthy(fields.get(field)))
}

/// Evaluate a single `field op value` condition string.
///
/// The parser is deliberately small: it covers the operators Frappe's condition
/// doctype offers and fails loudly on anything else. Evaluating arbitrary code
/// is exactly what this project does not do.
fn evaluate_condition(condition: &str, fields: &HashMap<String, Value>) -> Result<bool> {
    let mut parts = condition.splitn(3, char::is_whitespace);
    let first = parts.next().unwrap_or_default().trim();
    let second = parts.next().map(str::trim);
    let third = parts.next().map(str::trim);
    match (second, third) {
        (None, _) => Ok(truthy(fields.get(first))),
        (Some(operator), Some(expected)) => compare(fields.get(first), operator, expected),
        (Some(expected), None) => {
            // `field value` is not a condition; refusing it keeps the reader
            // from guessing what the author meant.
            Err(Error::Validation(ValidationError::Invalid(format!(
                "unsupported Document Naming Rule condition `{condition}` (expected `field op value`, got `{first} {expected}`)"
            ))))
        }
    }
}

fn compare(left: Option<&Value>, operator: &str, expected: &str) -> Result<bool> {
    let left = left.unwrap_or(&Value::Null);
    let expected = expected.trim().trim_matches(['"', '\'']);
    let left_text = crate::naming::text(left);
    let left_number = numeric(left);
    let right_number = expected.parse::<f64>().ok();
    let equals = left_text == expected
        || (left_number.is_some() && right_number.is_some() && left_number == right_number);
    let ordering = match (left_number, right_number) {
        (Some(left), Some(right)) => left.partial_cmp(&right),
        _ => Some(left_text.as_str().cmp(expected)),
    };
    match operator {
        "" | "=" | "==" => Ok(equals),
        "!=" | "<>" => Ok(!equals),
        ">" => Ok(ordering.is_some_and(|order| order.is_gt())),
        "<" => Ok(ordering.is_some_and(|order| order.is_lt())),
        ">=" => Ok(ordering.is_some_and(|order| order.is_ge())),
        "<=" => Ok(ordering.is_some_and(|order| order.is_le())),
        other => Err(Error::Validation(ValidationError::Invalid(format!(
            "unsupported Document Naming Rule operator `{other}`"
        )))),
    }
}

fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(flag) => Some(f64::from(u8::from(*flag))),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// Frappe truthiness for a condition operand: non-empty strings (except `0`
/// and `false`), non-zero numbers, `true`, non-empty collections.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => {
            !text.is_empty() && text != "0" && !text.eq_ignore_ascii_case("false")
        }
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

fn integer(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(text)) => text.parse().ok(),
        _ => None,
    }
}

/// Mint from a matched rule and persist the bumped counter on its own row.
///
/// The UPDATE runs on the caller's sink, inside the same transaction as the
/// document INSERT: a rolled-back insert must not burn a rule number.
async fn mint_from_rule(
    rule: &Value,
    row_name: &str,
    naming: &mut WriteSink<'_>,
) -> Result<String> {
    let prefix = rule.get("prefix").and_then(Value::as_str).unwrap_or("");
    // Frappe's current field is `prefix_digits`; older sites used `digits`.
    let digits = integer(rule.get("prefix_digits"))
        .or_else(|| integer(rule.get("digits")))
        .filter(|digits| *digits > 0)
        .and_then(|digits| usize::try_from(digits).ok())
        .unwrap_or(5);
    let counter = integer(rule.get("counter")).unwrap_or(0).saturating_add(1);
    let name = format!("{prefix}{counter:0digits$}");

    let quoted = q(naming.dialect(), &rule_table()?)?;
    naming
        .execute(&Stmt::new(
            format!("UPDATE {quoted} SET counter = ?, modified = ? WHERE name = ?"),
            vec![
                Value::from(counter),
                Value::from(chrono::Utc::now().timestamp()),
                Value::from(row_name),
            ],
        ))
        .await?;
    Ok(name)
}
