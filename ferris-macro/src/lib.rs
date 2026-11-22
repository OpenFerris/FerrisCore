//! Ferris Macro - Proc macros for the Ferris framework
//!
//! Provides #[doctype], #[workflow], #[permission], #[job] macros for compile-time code generation.
//!
//! This crate is deliberately thin: it holds the four entry points and the
//! module declarations. The attribute grammar lives in [`args`], the shared
//! helpers in [`util`], and each macro's expansion in its own module.

// A proc-macro crate may not export anything besides its `#[proc_macro*]` entry
// points, so an internal module can only be `pub(crate)`; clippy's
// `redundant_pub_crate` assumes `pub` is available and is wrong here.
#![allow(clippy::redundant_pub_crate)]

use proc_macro::TokenStream;
use syn::{ItemStruct, parse_macro_input};

use args::{DoctypeArgs, JobArgs, PermissionArgs, WorkflowArgs};

// A proc-macro crate may only export the `#[proc_macro*]` entry points, so
// these modules are `pub(crate)`: wide enough for the entry points to reach
// them, narrow enough that the compiler accepts the crate.
pub(crate) mod args;
pub(crate) mod doctype;
mod field;
mod job;
mod permission;
pub(crate) mod util;
mod workflow;

#[proc_macro_attribute]
pub fn doctype(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let args = parse_macro_input!(attr as DoctypeArgs);

    doctype::expand(args, input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

#[proc_macro_attribute]
pub fn workflow(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let args = parse_macro_input!(attr as WorkflowArgs);

    workflow::expand(args, input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

#[proc_macro_attribute]
pub fn permission(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let args = parse_macro_input!(attr as PermissionArgs);

    permission::expand(&args, &input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

#[proc_macro_attribute]
pub fn job(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let args = parse_macro_input!(attr as JobArgs);

    job::expand(&args, &input).into()
}
