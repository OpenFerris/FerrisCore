//! OpenAPI 3 document for the REST/JSON-RPC surface.
//!
//! DocTypes are dynamic, so the resource endpoints are described generically
//! with a `{doctype}` path parameter; the registered API methods are listed
//! under the `x-ferris-methods` vendor extension.

use serde_json::{Value, json};

use crate::module::OpenApiFragment;

/// Build an OpenAPI 3.0 document describing the Ferris API.
///
/// The document is assembled from [`paths`] and [`schemas`] rather than one
/// long literal, so no single function outgrows the readability limit
///.
pub fn openapi_spec(title: &str, version: &str, methods: &[String]) -> Value {
    json!({
        "openapi": "3.0.3",
        "info": {
            "title": title,
            "version": version,
            "description": "Ferris REST, JSON-RPC and batch API."
        },
        "x-ferris-methods": methods,
        "paths": paths(methods),
        "components": { "schemas": schemas() }
    })
}

/// Merge the paths contributed by native modules into a spec.
///
/// A module owns its paths: the host only inserts them, and a repeated path
/// merges an operation object (for example a module adding `post` to a path the
/// core already documents as `get`). The fragment keys are full paths, so the
/// mounted router and `/api/openapi.json` use the same names.
pub fn merge_fragments(spec: &mut Value, fragments: &[OpenApiFragment]) {
    let Some(paths) = spec.get_mut("paths").and_then(Value::as_object_mut) else {
        return;
    };
    for fragment in fragments {
        for (path, operations) in &fragment.paths {
            match paths.get_mut(path) {
                Some(existing) => {
                    if let (Some(existing), Some(operations)) =
                        (existing.as_object_mut(), operations.as_object())
                    {
                        for (method, operation) in operations {
                            existing.insert(method.clone(), operation.clone());
                        }
                    }
                }
                None => {
                    paths.insert(path.clone(), operations.clone());
                }
            }
        }
    }
}

/// The `paths` object: the generic resource, method and RPC endpoints, plus one
/// concrete `/api/method/{name}` entry per registered method for clients.
fn paths(methods: &[String]) -> Value {
    let mut paths = json!({
        "/api/resource/{doctype}": {
            "get": {
                "summary": "List documents of a DocType",
                "tags": ["resource"],
                "parameters": [path_param("doctype")],
                "responses": { "200": response_with("ListResponse") }
            },
            "post": {
                "summary": "Create a document",
                "tags": ["resource"],
                "parameters": [path_param("doctype")],
                "requestBody": json_body("Document"),
                "responses": { "201": response_with("Document") }
            }
        },
        "/api/resource/{doctype}/{name}": {
            "get": {
                "summary": "Get a document by name",
                "tags": ["resource"],
                "parameters": [path_param("doctype"), path_param("name")],
                "responses": { "200": response_with("Document"), "404": response_with("Error") }
            },
            "put": {
                "summary": "Update a document",
                "tags": ["resource"],
                "parameters": [path_param("doctype"), path_param("name")],
                "requestBody": json_body("Document"),
                "responses": { "200": response_with("Document") }
            },
            "delete": {
                "summary": "Delete a document",
                "tags": ["resource"],
                "parameters": [path_param("doctype"), path_param("name")],
                "responses": { "204": json!({ "description": "Deleted" }) }
            }
        },
        "/api/method/{method}": {
            "post": {
                "summary": "Call a registered API method",
                "tags": ["method"],
                "parameters": [path_param("method")],
                "requestBody": json_body("Document"),
                "responses": { "200": response_with("Document") }
            }
        },
        "/api/jsonrpc": {
            "post": {
                "summary": "JSON-RPC 2.0 endpoint",
                "tags": ["rpc"],
                "requestBody": json_body("JsonRpcRequest"),
                "responses": { "200": response_with("JsonRpcResponse") }
            }
        },
        "/api/batch": {
            "post": {
                "summary": "Batch JSON-RPC requests",
                "tags": ["rpc"],
                "requestBody": json!({
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": { "type": "array", "items": { "$ref": "#/components/schemas/JsonRpcRequest" } }
                        }
                    }
                }),
                "responses": { "200": response_with("JsonRpcResponse") }
            }
        },
        "/api/graphql": {
            "post": {
                "summary": "GraphQL endpoint",
                "tags": ["graphql"],
                "requestBody": json_body("GraphQlRequest"),
                "responses": { "200": response_with("GraphQlResponse") }
            }
        },
        "/api/openapi.json": {
            "get": {
                "summary": "This document",
                "tags": ["meta"],
                "responses": { "200": response_with("Document") }
            }
        }
    });

    // Add a concrete path for each registered method (nice for clients) while
    // keeping the generic `{method}` path above.
    if let Some(paths_obj) = paths.as_object_mut() {
        for method in methods {
            paths_obj.insert(
                format!("/api/method/{method}"),
                json!({
                    "post": {
                        "summary": format!("Call {method}"),
                        "tags": ["method"],
                        "requestBody": json_body("Document"),
                        "responses": { "200": response_with("Document") }
                    }
                }),
            );
        }
    }

    paths
}

/// The `components.schemas` object every response reference points at.
fn schemas() -> Value {
    json!({
                "Document": { "type": "object", "additionalProperties": true },
                "ListResponse": {
                    "type": "object",
                    "properties": {
                        "data": { "type": "array", "items": { "$ref": "#/components/schemas/Document" } },
                        "total": { "type": "integer", "format": "int64" },
                        "limit": { "type": "integer" },
                        "offset": { "type": "integer" }
                    }
                },
                "Error": {
                    "type": "object",
                    "properties": {
                        "error": { "type": "string" },
                        "message": { "type": "string" }
                    }
                },
                "JsonRpcRequest": {
                    "type": "object",
                    "required": ["method"],
                    "properties": {
                        "jsonrpc": { "type": "string" },
                        "method": { "type": "string" },
                        "params": { "$ref": "#/components/schemas/Document" },
                        "id": {}
                    }
                },
                "JsonRpcResponse": {
                    "type": "object",
                    "properties": {
                        "jsonrpc": { "type": "string" },
                        "result": {},
                        "error": { "$ref": "#/components/schemas/Error" },
                        "id": {}
                    }
                },
                "GraphQlRequest": {
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": { "type": "string" },
                        "variables": { "$ref": "#/components/schemas/Document" },
                        "operationName": { "type": "string" }
                    }
                },
                "GraphQlResponse": {
                    "type": "object",
                    "properties": {
                        "data": {},
                        "errors": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/Error" }
                        }
                    }
                }
    })
}

fn path_param(name: &str) -> Value {
    json!({
        "name": name,
        "in": "path",
        "required": true,
        "schema": { "type": "string" }
    })
}

fn response_with(schema: &str) -> Value {
    json!({
        "description": "OK",
        "content": {
            "application/json": {
                "schema": { "$ref": format!("#/components/schemas/{schema}") }
            }
        }
    })
}

fn json_body(schema: &str) -> Value {
    json!({
        "required": true,
        "content": {
            "application/json": {
                "schema": { "$ref": format!("#/components/schemas/{schema}") }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_spec_lists_endpoints_and_methods() {
        let spec = openapi_spec(
            "Ferris API",
            "0.1.0",
            &["ping".to_string(), "demo.echo".to_string()],
        );

        assert_eq!(spec["openapi"], "3.0.3");
        assert_eq!(spec["info"]["title"], "Ferris API");

        let paths = spec["paths"].as_object().unwrap();
        for path in [
            "/api/resource/{doctype}",
            "/api/resource/{doctype}/{name}",
            "/api/method/{method}",
            "/api/method/ping",
            "/api/method/demo.echo",
            "/api/jsonrpc",
            "/api/batch",
            "/api/openapi.json",
        ] {
            assert!(paths.contains_key(path), "missing path {path}");
        }

        assert_eq!(spec["x-ferris-methods"][0], "ping");
        assert!(spec["components"]["schemas"]["Document"].is_object());
    }
}
