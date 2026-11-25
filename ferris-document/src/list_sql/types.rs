use crate::filters::{FilterOp, ListFilter};
use ferris_core::Value;
use ferris_schema::FieldMeta;

/// The engine columns a `tab<DocType>` table always has.
///
/// Derived from [`ferris_schema::migration::document_columns`]; kept as an
/// explicit allow-list rather than derived at run time because this is the set
/// the SQL generation trusts, and a name that is not in it must fall back to
/// the Rust path instead of producing a statement against a column that does
/// not exist.
///
/// Declared fields own real columns too, but which ones exist is per-DocType
/// metadata ([`Plan::build`]'s `columns`) rather than a fixed list.
///
/// **Having a column is not the same as being able to filter on it** — see
/// [`FILTERABLE_FIELDS`].
pub const COLUMN_FIELDS: [&str; 8] = [
    "name",
    "owner",
    "docstatus",
    "creation",
    "modified",
    "modified_by",
    "amended_from",
    "workflow_state",
];

/// The system columns every `SELECT` projects, so the engine can rebuild the
/// document from them. Declared field columns are appended by [`Plan::build`].
pub const SELECT_COLUMNS: [&str; 8] = COLUMN_FIELDS;

/// Columns whose stored representation matches the document's.
///
/// `docstatus` and `creation`/`modified` are regular columns too, but their
/// column value is *not* the document string (`0/1/2` vs
/// `draft`/`submitted`/`cancelled`, epoch seconds vs RFC 3339). They are pushed
/// through [`represented_filter`], which converts the value first; this list
/// stays the set whose value needs no conversion.
pub const FILTERABLE_FIELDS: [&str; 2] = ["name", "owner"];

/// Columns backed by a real column with a different representation.
///
/// Keeping them out of [`FILTERABLE_FIELDS`] is what stops `render::sql_predicate`
/// from binding the document text against the column: the conversion lives in
/// [`represented_filter`], next to the only code that knows what the column
/// stores.
pub const REPRESENTED_FIELDS: [&str; 3] = ["docstatus", "creation", "modified"];

/// Whether `field` is backed by a real column, so it can be ordered in SQL.
pub fn is_column_field(field: &str) -> bool {
    COLUMN_FIELDS.contains(&field)
}

/// Whether `field`'s column holds the same value the document does, so it can be
/// filtered in SQL.
pub fn is_filterable_field(field: &str) -> bool {
    FILTERABLE_FIELDS.contains(&field)
}

/// A predicate the database can evaluate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Predicate {
    Eq {
        column: String,
        value: Value,
    },
    Like {
        column: String,
        needle: String,
    },
    /// `column IN (...)`. Values are typed because a declared field's column is
    /// `BIGINT`/`DOUBLE`/`BOOLEAN` as often as it is text, and binding `"1"`
    /// against `BIGINT` fails on PostgreSQL.
    In {
        column: String,
        values: Vec<Value>,
    },
    Gt {
        column: String,
        value: Value,
    },
    Gte {
        column: String,
        value: Value,
    },
    Lt {
        column: String,
        value: Value,
    },
    Lte {
        column: String,
        value: Value,
    },
}

/// A filter that could not be pushed down and must stay in Rust.
///
/// Recorded so the log can say *what* was left behind. A silently dropped
/// filter is a filter that quietly changes the result set, which is the failure
/// mode this module exists to avoid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Residual {
    pub field: String,
    pub reason: ResidualReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidualReason {
    /// The field has no column in the DocType's table.
    NotAColumn,
    /// The field has a column, but it stores the value in a different
    /// representation from the document — see [`FILTERABLE_FIELDS`].
    RepresentedDifferently,
    /// The operator has no portable SQL spelling, or its Rust semantics are
    /// string-coerced and would not survive the round trip.
    UnsupportedOperator,
    /// The filters form more than one OR group, so they are a disjunction and
    /// cannot be appended to a `WHERE` with `AND`.
    Disjunction,
    /// The requested ordering field has no column.
    OrderByNotAColumn,
    /// The filter value cannot be coerced to the declared column's type. A
    /// wrong-typed bound value either fails at the database or (worse) compares
    /// against a different domain than the Rust evaluator, so the filter stays
    /// in Rust instead.
    ValueNotCoercible,
    /// The equality uses `""`, which the Rust evaluator also matches for a
    /// missing cell. A column stores `NULL` for that case, and `column = ''`
    /// never matches `NULL`.
    EmptyStringNotPushed,
    /// The filter is a timestamp, and its comparison is precision-aware in
    /// Rust (`equals 12:34` matches `12:34:56`). A `Datetime` column stores the
    /// literal text, so the SQL comparison cannot reproduce that.
    TimestampNotPushed,
}

impl ResidualReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAColumn => "field has no column in the document table",
            Self::RepresentedDifferently => {
                "column stores this field in another representation than the document"
            }
            Self::UnsupportedOperator => "operator has no portable SQL equivalent",
            Self::Disjunction => "filters form an OR group; a WHERE can only AND",
            Self::OrderByNotAColumn => "order_by field has no column",
            Self::ValueNotCoercible => "filter value cannot be bound as the declared column's type",
            Self::EmptyStringNotPushed => {
                "empty equality also matches a missing cell in Rust, not a SQL NULL"
            }
            Self::TimestampNotPushed => {
                "timestamp comparison uses the filter's precision, which the text column cannot express"
            }
        }
    }
}

/// What an `ORDER BY` may name: a real column (system or declared).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OrderKey {
    Column(String),
}

/// The SQL half of a list request: a `WHERE` body, an `ORDER BY` and a page.
///
/// Deliberately *not* a general query builder. It builds exactly the one
/// statement shape `list()` needs, because a general builder would let a caller
/// name an arbitrary column and reintroduce the injection surface
/// [`Ident`] exists to close.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub(crate) predicates: Vec<Predicate>,
    pub(crate) order_by: Option<(OrderKey, bool)>,
    /// The columns the `SELECT` projects: the system columns plus every
    /// declared field column, so the engine can rebuild the document.
    pub(crate) projection: Vec<String>,
    /// `true` when the ORDER BY field had no column and ordering happens in Rust.
    pub(crate) order_in_rust: bool,
    /// How many filters stayed behind for the Rust filter to apply. Kept on the
    /// plan rather than only in the returned [`Residual`] list so
    /// [`Plan::is_complete`] cannot disagree with what `build` reported.
    pub(crate) residual: usize,
}

/// The `SMALLINT` a `docstatus` word is stored as.
///
/// Only the three words convert: the Rust evaluator compares the *document*
/// value, which is a word, so a numeric filter (`0`) matches nothing there and
/// must not start matching here. Ranges are not converted either — the Rust
/// comparison is lexicographic over the words, and the numeric order is a
/// different answer, not a faster one.
fn docstatus_code(value: &Value) -> Option<i64> {
    // Exact match only: the evaluator compares the stored word
    // case-sensitively, so `Draft` must stay in Rust (where it matches
    // nothing) instead of being normalised into a hit.
    match value.as_str()?.trim() {
        "draft" => Some(0),
        "submitted" => Some(1),
        "cancelled" => Some(2),
        _ => None,
    }
}

/// The `(start, span)` a timestamp filter names, in epoch seconds.
///
/// `span` is `None` for a second-precision needle, `Some(60)` for
/// `datetime-local`, and `Some(86_400)` for a date. Rust compares at the
/// needle's precision (`filters::timestamp_cmp`), so a day filter means the
/// whole day: representing it as a half-open `[start, start + span)` range is
/// the exact SQL equivalent, not an approximation.
fn timestamp_range(text: &str) -> Option<(i64, Option<i64>)> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime};

    const NAIVE: &[(&str, Option<i64>)] = &[
        ("%Y-%m-%dT%H:%M:%S%.f", None),
        ("%Y-%m-%dT%H:%M:%S", None),
        ("%Y-%m-%dT%H:%M", Some(60)),
        ("%Y-%m-%d %H:%M:%S%.f", None),
        ("%Y-%m-%d %H:%M:%S", None),
        ("%Y-%m-%d %H:%M", Some(60)),
        ("%Y-%m-%d", Some(86_400)),
    ];

    let text = text.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        // `filters` keeps the written wall clock (`naive_local`) rather than
        // converting to UTC. Engine-stamped timestamps are UTC, so treating the
        // wall clock as UTC reproduces the same instant.
        return Some((dt.naive_local().and_utc().timestamp(), None));
    }
    for (format, span) in NAIVE {
        if let Ok(dt) = NaiveDateTime::parse_from_str(text, format) {
            return Some((dt.and_utc().timestamp(), *span));
        }
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|dt| (dt.and_utc().timestamp(), Some(86_400)))
}

/// SQL predicates for one filter over a [`REPRESENTED_FIELDS`] column.
///
/// `None` means the filter stays in Rust, either because the operator has no
/// faithful SQL spelling (`like`/`in` over a timestamp) or because the value is
/// not one this module can convert. Keeping the fallback visible is what stops
/// a conversion bug from silently shrinking a result set.
pub(super) fn represented_filter(filter: &ListFilter) -> Option<Vec<Predicate>> {
    if !REPRESENTED_FIELDS.contains(&filter.field.as_str()) {
        return None;
    }
    match filter.field.as_str() {
        "docstatus" => {
            if filter.op != FilterOp::Equals {
                return None;
            }
            let code = docstatus_code(&filter.value)?;
            Some(vec![Predicate::Eq {
                column: "docstatus".to_string(),
                value: Value::from(code),
            }])
        }
        "creation" | "modified" => {
            let (start, span) = timestamp_range(filter.value.as_str()?)?;
            let column = filter.field.clone();
            Some(match (filter.op, span) {
                (FilterOp::Like | FilterOp::In, _) => return None,
                // A day/minute filter names a bucket. The bucket is exactly a
                // half-open range, so these predicates keep the Rust answer.
                (FilterOp::Equals, Some(span)) => vec![
                    Predicate::Gte {
                        column: column.clone(),
                        value: Value::from(start),
                    },
                    Predicate::Lt {
                        column,
                        value: Value::from(start + span),
                    },
                ],
                (FilterOp::Equals, None) => vec![Predicate::Eq {
                    column,
                    value: Value::from(start),
                }],
                (FilterOp::Gt, Some(span)) => vec![Predicate::Gte {
                    column,
                    value: Value::from(start + span),
                }],
                (FilterOp::Gt, None) => vec![Predicate::Gt {
                    column,
                    value: Value::from(start),
                }],
                (FilterOp::Gte, _) => vec![Predicate::Gte {
                    column,
                    value: Value::from(start),
                }],
                (FilterOp::Lt, _) => vec![Predicate::Lt {
                    column,
                    value: Value::from(start),
                }],
                (FilterOp::Lte, Some(span)) => vec![Predicate::Lt {
                    column,
                    value: Value::from(start + span),
                }],
                (FilterOp::Lte, None) => vec![Predicate::Lte {
                    column,
                    value: Value::from(start),
                }],
            })
        }
        _ => None,
    }
}

/// The legacy equality-map spelling of a [`REPRESENTED_FIELDS`] predicate.
///
/// Only `docstatus` converts safely here: the map compares values structurally,
/// so a `creation` string matches the stored RFC 3339 text exactly and an epoch
/// column predicate would admit a row whose text differs. That case stays in
/// Rust on purpose.
pub(super) fn represented_equality(field: &str, value: &Value) -> Option<Value> {
    if field != "docstatus" {
        return None;
    }
    docstatus_code(value).map(Value::from)
}

/// The declared field `field`, if the `DocType` gave it a physical column.
///
/// `columns` is the caller's [`super::Plan::build`] argument: the stored
/// fields for this `DocType` and dialect (`engine::support::stored_fields`).
/// A declared field that is layout-only, virtual or a child table is absent,
/// so it correctly falls back to the Rust path.
pub(super) fn declared_column<'a>(columns: &'a [FieldMeta], field: &str) -> Option<&'a FieldMeta> {
    columns.iter().find(|column| column.fieldname == field)
}

/// A field's canonical fieldtype, matching what the schema used to pick its
/// column type.
fn fieldtype(field: &FieldMeta) -> &str {
    ferris_schema::fieldtype::canonical(&field.fieldtype).unwrap_or_else(|| field.fieldtype.trim())
}

/// Whether the column stores a JSON number that SQL compares numerically.
pub(super) fn is_numeric_column(field: &FieldMeta) -> bool {
    matches!(fieldtype(field), "Int" | "Float" | "Currency" | "Percent")
}

/// Whether the column stores text, so `LIKE` has the Rust evaluator's meaning.
pub(super) fn is_text_column(field: &FieldMeta) -> bool {
    !is_numeric_column(field) && fieldtype(field) != "Check"
}

fn int_value(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_str()
            .and_then(|text| text.trim().parse::<i64>().ok())
    })
}

fn float_value(value: &Value) -> Option<f64> {
    let number = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))?;
    number.is_finite().then_some(number)
}

fn bool_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        // `parse_list_filters` hands the needle over as text, and `cell_str`
        // renders a stored boolean as exactly `true`/`false`. Any other
        // spelling (`1`, `yes`, `True`) matches nothing in Rust, so binding it
        // as a boolean would start matching rows the evaluator rejects.
        Value::String(text) if text == "true" => Some(true),
        Value::String(text) if text == "false" => Some(false),
        _ => None,
    }
}

fn text_value(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        // `column_value` stores every non-string as its JSON text; the filter
        // side uses the same representation so the comparison stays exact.
        other => Some(other.to_json().to_string()),
    }
}

/// A filter value as the value its declared column binds.
///
/// `None` means the value cannot be represented in the column's type. The
/// caller keeps that filter in Rust rather than binding a `TEXT` value to a
/// `BIGINT` column (which PostgreSQL rejects) or a value from the wrong domain
/// (which would silently change the row set).
pub(super) fn coerce_column_value(field: &FieldMeta, value: &Value) -> Option<Value> {
    match fieldtype(field) {
        "Int" => int_value(value).map(Value::from),
        "Float" | "Currency" | "Percent" => float_value(value).map(Value::from),
        "Check" => bool_value(value).map(Value::from),
        _ => text_value(value).map(Value::from),
    }
}

/// Whether an equality is the `""` the Rust evaluator also matches for a
/// missing cell.
pub(super) fn is_empty_string(value: &Value) -> bool {
    value.as_str().is_some_and(str::is_empty)
}

fn is_date_only(text: &str) -> bool {
    chrono::NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").is_ok()
}

/// Whether a precision-aware Rust timestamp comparison would differ from the
/// column's text comparison.
///
/// `Date` columns are validated to date-only text, so a date-only needle is
/// the same comparison in both places. Every other timestamp value (and every
/// `Datetime`, whose stored offsets/spellings text does not order like an
/// instant) stays in Rust.
pub(super) fn timestamp_guard(
    field: &FieldMeta,
    value: &Value,
) -> std::result::Result<(), ResidualReason> {
    if !is_text_column(field) {
        // A numeric/boolean column cannot round-trip a timestamp at all; the
        // type coercion below is the honest reason to report.
        return Ok(());
    }
    let Some(text) = value.as_str() else {
        return Ok(());
    };
    if !crate::filters::is_timestamp_value(text) {
        return Ok(());
    }
    if fieldtype(field) == "Date" && is_date_only(text) {
        return Ok(());
    }
    Err(ResidualReason::TimestampNotPushed)
}

/// The legacy equality-map spelling of a declared column predicate.
pub(super) fn declared_equality(
    field: &FieldMeta,
    value: &Value,
) -> std::result::Result<Value, ResidualReason> {
    if is_empty_string(value) {
        return Err(ResidualReason::EmptyStringNotPushed);
    }
    // The map compares values structurally, so a date/time string matches the
    // column's literal text; only the operator path has the precision-aware
    // timestamp comparison.
    coerce_column_value(field, value).ok_or(ResidualReason::ValueNotCoercible)
}
