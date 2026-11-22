use ferris_core::Value;

/// The columns a `tab<DocType>` table really has.
///
/// Derived from [`ferris_schema::migration::document_columns`]; kept as an
/// explicit allow-list rather than derived at run time because this is the set
/// the SQL generation trusts, and a name that is not in it must fall back to
/// the Rust path instead of producing a statement against a column that does
/// not exist.
///
/// **Having a column is not the same as being able to filter on it** — see
/// [`FILTERABLE_FIELDS`].
pub const COLUMN_FIELDS: [&str; 5] = ["name", "owner", "docstatus", "creation", "modified"];

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
    /// Exact value of `data.field` for a field the DocType declares indexed.
    JsonEq {
        field: String,
        value: Value,
    },
    /// `data.field` is one of a list; the same declaration as [`Self::JsonEq`].
    JsonIn {
        field: String,
        values: Vec<String>,
    },
    Like {
        column: String,
        needle: String,
    },
    In {
        column: String,
        values: Vec<String>,
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
    /// The field lives inside the `data` blob and has no column.
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
    /// `LIKE` over an indexed JSON field. The index is exact-text, so a
    /// leading wildcard cannot use it, and the Rust substring match has no
    /// need to be duplicated in SQL — it stays in Rust.
    JsonLikeNotPushed,
    /// A range (`gt`/`gte`/`lt`/`lte`) over an indexed JSON field. The Rust
    /// comparator (`filters::cmp`) is numeric when both sides parse as a
    /// number and case-insensitive otherwise; SQL text comparison would return
    /// a different set, so ranges are never pushed even for indexed fields.
    JsonRangeNotPushed,
    /// An exact (`=`/`in`) filter over an indexed JSON field whose *value*
    /// the SQL round trip cannot reproduce exactly. The Rust evaluator reads a
    /// missing cell as `""` (`cell_str`) and coerces numbers/booleans to text;
    /// a SQL equality against the JSON expression instead sees `NULL` for an
    /// absent field and the raw JSON type otherwise. A timestamp has the same
    /// problem: the evaluator compares it at the precision the filter names,
    /// while the index compares the literal text. Pushing it would narrow the
    /// result set the Rust filter would have kept, so it stays in Rust.
    JsonExactNotPushed,
}

impl ResidualReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAColumn => "field has no column; it lives in the document blob",
            Self::RepresentedDifferently => {
                "column stores this field in another representation than the document"
            }
            Self::UnsupportedOperator => "operator has no portable SQL equivalent",
            Self::Disjunction => "filters form an OR group; a WHERE can only AND",
            Self::OrderByNotAColumn => "order_by field has no column",
            Self::JsonLikeNotPushed => {
                "LIKE over an indexed JSON field cannot use its exact-text index"
            }
            Self::JsonRangeNotPushed => {
                "range over an indexed JSON field would change Rust comparison semantics"
            }
            Self::JsonExactNotPushed => {
                "exact match over an indexed JSON field would change Rust equality semantics"
            }
        }
    }
}

/// What an `ORDER BY` may name: a real column, or a field the DocType declares
/// indexed so the database can order by its JSON value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OrderKey {
    Column(String),
    JsonField(String),
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
pub(super) fn represented_filter(filter: &crate::filters::ListFilter) -> Option<Vec<Predicate>> {
    use crate::filters::FilterOp;

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
