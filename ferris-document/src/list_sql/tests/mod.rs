use super::*;
use ferris_schema::contract::list::FilterOp;
use std::collections::HashMap;

// Re-exported so `use super::*;` in `pushdown`/`index` picks these up; a glob
// import never reports `unused_imports`, so the group files stay header-free.
pub(crate) use crate::filters::ListFilter;
pub(crate) use ferris_core::Value;
pub(crate) use ferris_db::Dialect;

fn filters(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), Value::string(*v)))
        .collect()
}

fn op(field: &str, op: FilterOp, value: &str) -> ListFilter {
    ListFilter {
        field: field.to_string(),
        op,
        value: Value::string(value),
        or: false,
    }
}

mod index;
mod pushdown;
