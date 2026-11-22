use super::render::*;
use super::types::*;
use crate::filters::ListFilter;
use ferris_core::{Result, Value};
use ferris_db::sql::{Ident, json_field_target};
use ferris_db::{Dialect, Stmt};

impl Plan {
    /// Build a plan from the column- and index-backed part of a query.
    ///
    /// `json_indexed_fields` are the document fields this DocType declares a
    /// single-field index over ([`DoctypeMeta::json_indexed_fields`]). For them
    /// the plan may push **exact** predicates (`=`, `in`) and ordering into SQL,
    /// because `data.field` is what the index is built on; everything else it
    /// returns as [`Residual`] for the caller to evaluate in Rust.
    ///
    /// Returns the plan plus every filter it could not represent, so the caller
    /// can keep evaluating those in Rust and log them.
    pub fn build(
        filters: &std::collections::HashMap<String, Value>,
        filter_ops: &[ListFilter],
        search: Option<&str>,
        order_by: Option<&str>,
        json_indexed_fields: &[String],
    ) -> (Self, Vec<Residual>) {
        let mut plan = Plan::default();
        let mut residual = Vec::new();
        let is_json_indexed = |field: &str| json_indexed_fields.iter().any(|f| f == field);

        for (field, value) in filters {
            if is_json_indexed(field) {
                // The hash map is the legacy REST spelling of an equality, and
                // it hands the value over as a typed `Value`. Only a non-empty
                // `String` compares the same in SQL as in Rust: the expression
                // index holds exact text, while the Rust path coerces cells
                // with `cell_str` (a missing cell is `""`) and compares
                // numbers against strings structurally. A number, a boolean or
                // an empty string would narrow the result set instead of
                // filtering it, so those stay in Rust and the plan is honest
                // that they did.
                match value {
                    Value::String(s) if !s.is_empty() => {
                        plan.predicates.push(Predicate::JsonEq {
                            field: field.clone(),
                            value: value.clone(),
                        });
                    }
                    _ => residual.push(Residual {
                        field: field.clone(),
                        reason: ResidualReason::JsonExactNotPushed,
                    }),
                }
                continue;
            }
            if let Some(converted) = represented_equality(field, value) {
                plan.predicates.push(Predicate::Eq {
                    column: field.clone(),
                    value: converted,
                });
                continue;
            }
            match pushdown_reason(field) {
                None => plan.predicates.push(Predicate::Eq {
                    column: field.clone(),
                    value: value.clone(),
                }),
                Some(reason) => residual.push(Residual {
                    field: field.clone(),
                    reason,
                }),
            }
        }

        // `matches_filters` ANDs the filters inside a group and ORs the groups
        // together, splitting whenever a filter carries `or`. Only a single
        // group is a plain conjunction, and only a plain conjunction can be
        // appended to a `WHERE` with `AND` — pushing one term of a disjunction
        // would turn "A OR B" into "A AND B" and drop every row that matched
        // only through the other side.
        let groups = group_count(filter_ops);
        if groups > 1 {
            for f in filter_ops {
                residual.push(Residual {
                    field: f.field.clone(),
                    reason: ResidualReason::Disjunction,
                });
            }
        } else {
            for f in filter_ops {
                if let Some(predicates) = represented_filter(f) {
                    plan.predicates.extend(predicates);
                    continue;
                }
                match sql_predicate(f, json_indexed_fields) {
                    Ok(Some(p)) => plan.predicates.push(p),
                    Ok(None) => residual.push(Residual {
                        field: f.field.clone(),
                        reason: ResidualReason::UnsupportedOperator,
                    }),
                    Err(reason) => residual.push(Residual {
                        field: f.field.clone(),
                        reason,
                    }),
                }
            }
        }

        // A substring search walks every string leaf of the document, so it is
        // a blob predicate by construction and can never be pushed down. It is
        // recorded so the reason is visible rather than inferred.
        if let Some(needle) = search {
            if !needle.trim().is_empty() {
                residual.push(Residual {
                    field: "<search>".to_string(),
                    reason: ResidualReason::NotAColumn,
                });
            }
        }

        match order_by {
            Some(expr) if !expr.trim().is_empty() => {
                let mut tokens = expr.split_whitespace();
                let field = tokens.next().unwrap_or_default();
                let desc = tokens
                    .next()
                    .map(|t| t.eq_ignore_ascii_case("desc"))
                    .unwrap_or(false);
                if is_column_field(field) {
                    plan.order_by = Some((OrderKey::Column(field.to_string()), desc));
                } else if is_json_indexed(field) {
                    // The DocType declared an index over this blob field, so the
                    // database can order by its JSON value like it orders by a
                    // column. Missing/null values sort last in both directions,
                    // matching `filters::order_documents`.
                    plan.order_by = Some((OrderKey::JsonField(field.to_string()), desc));
                } else {
                    plan.order_in_rust = true;
                    residual.push(Residual {
                        field: field.to_string(),
                        reason: ResidualReason::OrderByNotAColumn,
                    });
                }
            }
            // No `order_by`: newest first, which is what the Desk expects and
            // what the engine used to hard-code as `ORDER BY modified DESC`.
            // `modified` is a column, so the default order is one SQL can serve
            // — and one `idx_modified` can use, which the previous unpaged
            // `SELECT` could not exploit because it sorted after materializing
            // every row.
            _ => {}
        }
        if plan.order_by.is_none() && !plan.order_in_rust {
            plan.order_by = Some((OrderKey::Column("modified".to_string()), true));
        }

        plan.residual = residual.len();
        (plan, residual)
    }

    /// Whether the plan narrows the row set at all.
    ///
    /// Ordering does not count: a plan with no predicates still carries the
    /// default `modified DESC` so SQL can serve the sort.
    pub fn filters_nothing(&self) -> bool {
        self.predicates.is_empty()
    }

    /// Whether ordering still has to happen in Rust.
    pub fn orders_in_rust(&self) -> bool {
        self.order_in_rust
    }

    /// Whether SQL saw *every* filter in the request.
    ///
    /// `false` whenever anything was left in [`Residual`]. Callers must not
    /// trust `COUNT(*)` for `total` in that case, because the count is over the
    /// pushed-down subset only. The leftover count is carried on the plan
    /// rather than recomputed from the [`Residual`] list so the two can never
    /// disagree.
    pub fn is_complete(&self) -> bool {
        self.residual == 0
    }

    /// How many filters stayed in Rust.
    pub fn residual_count(&self) -> usize {
        self.residual
    }

    /// Add an extra `WHERE` term the caller supplies, already trusted.
    ///
    /// Used for the engine's own `owner` narrowing when the policy reduces to
    /// "rows this caller owns". The column name is validated here rather than
    /// at the call site so there is one place that can emit it.
    pub fn with_equality(mut self, column: &str, value: Value) -> Result<Self> {
        Ident::new(column)?;
        self.predicates.push(Predicate::Eq {
            column: column.to_string(),
            value,
        });
        Ok(self)
    }

    /// Add `column IN (values)`, for a predicate the caller resolved elsewhere.
    ///
    /// [`crate::ListQuery::only_names`] is the case that exists for: an API
    /// layer that keeps its own side table (`only_liked`) hands the engine a
    /// decision it cannot express as a document filter, and the engine turns it
    /// into a column predicate instead of re-filtering in Rust.
    pub fn with_in(mut self, column: &str, values: Vec<String>) -> Result<Self> {
        Ident::new(column)?;
        self.predicates.push(Predicate::In {
            column: column.to_string(),
            values,
        });
        Ok(self)
    }

    /// Render the `WHERE` body with its parameters, or `None` when empty.
    fn where_clause(&self, dialect: Dialect, params: &mut Vec<Value>) -> Result<Option<String>> {
        if self.predicates.is_empty() {
            return Ok(None);
        }
        let mut terms = Vec::with_capacity(self.predicates.len());
        for predicate in &self.predicates {
            terms.push(render_predicate(predicate, dialect, params)?);
        }
        Ok(Some(format!(" WHERE {}", terms.join(" AND "))))
    }

    /// Render the `ORDER BY` body, or `None` to leave the database's order.
    fn order_clause(&self, dialect: Dialect) -> Result<Option<String>> {
        let Some((key, desc)) = &self.order_by else {
            return Ok(None);
        };
        let direction = if *desc { "DESC" } else { "ASC" };
        Ok(Some(match key {
            OrderKey::Column(field) => {
                let column = Ident::new(field)?.quote(dialect);
                format!(" ORDER BY {column} {direction}")
            }
            OrderKey::JsonField(field) => {
                let target = json_field_target(dialect, field)?;
                // `order_documents` sorts an empty/missing cell *last* in both
                // directions. A blob field has no cell for a row that omits it,
                // which the expression yields as SQL NULL; ordering on
                // `IS NULL` first reproduces "empty last" for ASC and DESC
                // alike without a dialect-specific `NULLS LAST` spelling.
                format!(" ORDER BY ({target}) IS NULL ASC, {target} {direction}")
            }
        }))
    }

    /// The `ORDER BY` for a paging query: the requested order (or the engine's
    /// default) with `name` appended as the final key so the order is total.
    ///
    /// `LIMIT`/`OFFSET` has no notion of a set, so without a total order two
    /// identical requests over unchanged data can return the same row twice or
    /// skip one. The tiebreaker has to be the `name` **column** of the doctype,
    /// not the table identifier: the table name is not a column of the projected
    /// relation, so PostgreSQL rejects it (`column "tabX" does not exist`) and
    /// SQLite/MySQL treat the double-quoted token as a string constant, leaving
    /// the order partial on both. When the caller already ordered by `name`,
    /// that single key is total and appending it again would only read as if
    /// there were two.
    fn paging_order_clause(&self, dialect: Dialect) -> Result<String> {
        if matches!(&self.order_by, Some((OrderKey::Column(field), _)) if field == "name") {
            return Ok(self.order_clause(dialect)?.unwrap_or_default());
        }
        let tiebreaker = Ident::new("name")?.quote(dialect);
        match self.order_clause(dialect)? {
            Some(order) => Ok(format!("{order}, {tiebreaker} ASC")),
            None => Ok(format!(" ORDER BY {tiebreaker} ASC")),
        }
    }

    /// The paged `SELECT` for a plan whose every filter reached SQL.
    ///
    /// `LIMIT`/`OFFSET` are bound, not interpolated: they arrive as `u32` from
    /// a request body, and an interpolated integer would still be a value
    /// spliced into SQL text by a route that has been wrong about bodies before.
    ///
    /// Call this only when [`Plan::is_complete`] is `true`. With a residual
    /// filter outstanding it would page over the wrong row set and report a
    /// `total` that counts rows the caller cannot see.
    pub fn select_page(
        &self,
        table: &str,
        dialect: Dialect,
        limit: u32,
        offset: u32,
    ) -> Result<Stmt> {
        let mut params = Vec::new();
        let quoted = Ident::new(table)?.quote(dialect);
        let where_sql = self.where_clause(dialect, &mut params)?;
        // `name` breaks ties so paging is stable: without a total order two
        // requests over unchanged data can return the same row twice or skip
        // one, because `LIMIT`/`OFFSET` has no notion of a set.
        let order = self.paging_order_clause(dialect)?;
        let sql = format!(
            "SELECT data, owner FROM {quoted}{where_sql}{order} LIMIT ? OFFSET ?",
            where_sql = where_sql.unwrap_or_default(),
        );
        params.push(Value::from(i64::from(limit)));
        params.push(Value::from(i64::from(offset)));
        Ok(Stmt::new(sql, params))
    }

    /// The unpaged `SELECT` for a plan that must still be filtered in Rust.
    ///
    /// No `LIMIT`: the engine is going to drop rows it may not read, so a limit
    /// applied here would shorten a page that the Rust filter then shrinks
    /// further. This is the honest shape for that case and it is why
    /// [`Plan::is_complete`] has to be answered before paging.
    ///
    /// It does carry the `ORDER BY`, and it has to: the Rust path filters and
    /// then takes a page, so the rows it is handed must arrive in a defined
    /// order — without one, two identical requests over unchanged data page
    /// differently, which is the same class of bug as an unstable `LIMIT`.
    pub fn select_all(&self, table: &str, dialect: Dialect) -> Result<Stmt> {
        let mut params = Vec::new();
        let quoted = Ident::new(table)?.quote(dialect);
        let where_sql = self.where_clause(dialect, &mut params)?;
        let order = self.paging_order_clause(dialect)?;
        let sql = format!(
            "SELECT data, owner FROM {quoted}{where_sql}{order}",
            where_sql = where_sql.unwrap_or_default()
        );
        Ok(Stmt::new(sql, params))
    }

    /// The total row count for a plan whose every filter reached SQL.
    pub fn count(&self, table: &str, dialect: Dialect) -> Result<Stmt> {
        let mut params = Vec::new();
        let quoted = Ident::new(table)?.quote(dialect);
        let where_sql = self.where_clause(dialect, &mut params)?;
        let sql = format!(
            "SELECT COUNT(*) AS n FROM {quoted}{where_sql}",
            where_sql = where_sql.unwrap_or_default()
        );
        Ok(Stmt::new(sql, params))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::FilterOp;

    fn filter(field: &str, op: FilterOp, value: &str) -> ListFilter {
        ListFilter {
            field: field.to_string(),
            op,
            value: Value::string(value),
            or: false,
        }
    }

    /// PERF-05: the Desk filter is written in document spelling (`submitted`),
    /// the column stores `1`. Binding the word would return no rows at all.
    #[test]
    fn a_docstatus_filter_is_bound_as_the_smallint() {
        let (plan, residual) = Plan::build(
            &std::collections::HashMap::new(),
            &[filter("docstatus", FilterOp::Equals, "submitted")],
            None,
            None,
            &[],
        );
        assert!(residual.is_empty(), "{residual:?}");
        let stmt = plan.select_page("tabToDo", Dialect::SQLite, 20, 0).unwrap();
        assert!(stmt.sql().contains("\"docstatus\" = ?"), "{}", stmt.sql());
        assert_eq!(stmt.params()[0], Value::from(1i64));
    }

    /// Case matters: the evaluator compares the stored word exactly, so a
    /// `Draft` filter matches nothing in Rust and must not become a hit here.
    #[test]
    fn a_docstatus_filter_with_other_case_stays_in_rust() {
        let (plan, residual) = Plan::build(
            &std::collections::HashMap::new(),
            &[filter("docstatus", FilterOp::Equals, "Draft")],
            None,
            None,
            &[],
        );
        assert_eq!(residual.len(), 1);
        assert!(!plan.is_complete());
    }

    /// A lifecycle range stays in Rust: the words' order is alphabetical, the
    /// column's is numeric, and the two answers differ.
    #[test]
    fn a_docstatus_range_is_not_pushed_as_a_number() {
        let (plan, residual) = Plan::build(
            &std::collections::HashMap::new(),
            &[filter("docstatus", FilterOp::Gt, "draft")],
            None,
            None,
            &[],
        );
        assert_eq!(residual.len(), 1);
        assert!(!plan.is_complete());
    }

    /// A date filter names the whole day; the half-open range is the exact SQL
    /// equivalent of the Rust precision-aware comparison.
    #[test]
    fn a_day_filter_becomes_the_exact_bucket_range() {
        let (plan, residual) = Plan::build(
            &std::collections::HashMap::new(),
            &[filter("creation", FilterOp::Equals, "2026-01-01")],
            None,
            None,
            &[],
        );
        assert!(residual.is_empty(), "{residual:?}");
        let stmt = plan.count("tabToDo", Dialect::SQLite).unwrap();
        assert!(stmt.sql().contains("\"creation\" >= ?"), "{}", stmt.sql());
        assert!(stmt.sql().contains("\"creation\" < ?"), "{}", stmt.sql());
        let start = stmt.params()[0].as_i64().unwrap();
        assert_eq!(stmt.params()[1].as_i64().unwrap(), start + 86_400);
    }

    /// `like` over a timestamp has no faithful SQL spelling and must not be
    /// silently dropped: it is reported as residual and stays in Rust.
    #[test]
    fn a_timestamp_like_stays_in_rust() {
        let (plan, residual) = Plan::build(
            &std::collections::HashMap::new(),
            &[filter("modified", FilterOp::Like, "2026-01")],
            None,
            None,
            &[],
        );
        assert_eq!(residual.len(), 1);
        assert!(!plan.is_complete());
    }
}
