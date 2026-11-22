//! A small GraphQL endpoint.
//!
//! A full GraphQL engine (`async-graphql`) is not available offline, so this is
//! a documented subset that covers the auto-generated read schema:
//!
//! ```graphql
//! { ToDo { name title } }                       # list a DocType
//! { ToDo(name: "TODO-1") { name title } }       # fetch one document
//! { ToDo(limit: 5, offset: 10) { name } }       # paginate
//! { __schema { types } }                        # introspection (DocTypes)
//! ```
//!
//! Aliases (`alias: field`) and variables (`$name`) are supported. Mutations and
//! subscriptions are rejected.
//!
//! The subset is split along the pipeline — [`wire`] for the HTTP envelope,
//! [`parser`] for the lexer/recursive-descent parser, [`execution`] for
//! resolution and projection, [`handler`] for the axum entry point — so the
//! nesting bound the parser enforces sits with the recursion it protects.

mod execution;
mod handler;
mod parser;
#[cfg(test)]
mod tests;
mod wire;

pub use execution::execute_query;
pub(crate) use handler::graphql_handler;
pub use wire::{GraphQlError, GraphQlRequest, GraphQlResponse};
