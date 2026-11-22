//! Query execution: root resolution, pagination clamps, projection and
//! introspection.
//!
//! Every root field is a `DocType` name, so the resolver is the generic
//! [`DocStore`] path: a read that goes through the same permission checks as
//! the REST layer instead of a second, unchecked data path.

use ferris_db::Db;
use serde_json::{Map as JsonMap, Value as JsonValue, json};

use crate::store::DocStore;
use crate::types::ListParams;

use super::parser::{Field, Parser, tokenize};
use super::wire::{GraphQlError, GraphQlResponse};

/// Parse and execute a GraphQL query against the document store.
pub async fn execute_query(
    pool: &Db,
    query: &str,
    variables: &JsonMap<String, JsonValue>,
) -> GraphQlResponse {
    let tokens = match tokenize(query) {
        Ok(tokens) => tokens,
        Err(message) => return GraphQlResponse::error(message),
    };
    let mut parser = Parser::new(tokens, variables.clone());
    let fields = match parser.parse_document() {
        Ok(fields) => fields,
        Err(message) => return GraphQlResponse::error(message),
    };

    let mut data = JsonMap::new();
    let mut errors = Vec::new();

    for field in &fields {
        let key = field.alias.clone().unwrap_or_else(|| field.name.clone());
        match resolve_root(pool, field).await {
            Ok(value) => {
                data.insert(key, value);
            }
            Err(message) => {
                errors.push(GraphQlError {
                    message,
                    path: Some(vec![key.clone()]),
                });
                data.insert(key, JsonValue::Null);
            }
        }
    }

    GraphQlResponse {
        data: Some(JsonValue::Object(data)),
        errors,
    }
}

async fn resolve_root(pool: &Db, field: &Field) -> std::result::Result<JsonValue, String> {
    match field.name.as_str() {
        "__typename" => return Ok(json!("Query")),
        "__schema" => return Ok(schema_introspection()),
        _ => {}
    }

    let doctype = field.name.clone();
    let arg = |name: &str| {
        field
            .args
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };

    if let Some(JsonValue::String(name)) = arg("name") {
        let document = DocStore::get(pool, &doctype, &name)
            .await
            .map_err(|e| crate::rpc::safe_error_message(&e))?;
        return Ok(project(document.to_json(), &field.selection, &doctype));
    }

    let mut params = ListParams::default();
    if let Some(JsonValue::Number(n)) = arg("limit") {
        // The engine materialises and orders every matching row before
        // applying `limit`, so keep client-supplied page sizes bounded
        // (same clamp as `DocStore::list`/`list_requested`) (§M13).
        params.limit = n
            .as_u64()
            .map(|v| u32::try_from(v).unwrap_or(u32::MAX).clamp(1, 1000));
    }
    if let Some(JsonValue::Number(n)) = arg("offset") {
        // A giant offset forces the engine to skip that many materialised
        // rows; cap it so pagination cannot become an amplification factor.
        params.offset = n
            .as_u64()
            .map(|v| u32::try_from(v).unwrap_or(u32::MAX).min(100_000));
    }

    let response = DocStore::list(pool, &doctype, &params)
        .await
        .map_err(|e| crate::rpc::safe_error_message(&e))?;
    let items: Vec<JsonValue> = response
        .data
        .into_iter()
        .map(|value| project(value.to_json(), &field.selection, &doctype))
        .collect();
    Ok(JsonValue::Array(items))
}

fn schema_introspection() -> JsonValue {
    let types: Vec<JsonValue> = ferris_schema::list_doctypes()
        .into_iter()
        .map(|name| json!({ "name": name.replace(' ', ""), "kind": "OBJECT" }))
        .collect();
    json!({
        "queryType": { "name": "Query" },
        "mutationType": JsonValue::Null,
        "subscriptionType": JsonValue::Null,
        "types": types,
    })
}

fn project(value: JsonValue, selection: &[Field], doctype: &str) -> JsonValue {
    if selection.is_empty() {
        return value;
    }
    match value {
        JsonValue::Object(map) => {
            let mut out = JsonMap::new();
            for field in selection {
                let key = field.alias.clone().unwrap_or_else(|| field.name.clone());
                if field.name == "__typename" {
                    out.insert(key, json!(doctype));
                    continue;
                }
                let child = map.get(&field.name).cloned().unwrap_or(JsonValue::Null);
                out.insert(key, project(child, &field.selection, doctype));
            }
            JsonValue::Object(out)
        }
        JsonValue::Array(items) => JsonValue::Array(
            items
                .into_iter()
                .map(|item| project(item, selection, doctype))
                .collect(),
        ),
        other => other,
    }
}
