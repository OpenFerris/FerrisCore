//! Helpers shared by more than one attribute macro.
//!
//! `parse_condition` and `to_snake_case` used to be copy-pasted in `workflow.rs`
//! and `permission.rs`. A fix to one (the escaped-string bug covered below)
//! silently missed the other, so the grammar and the generated-name rule live
//! here now and have exactly one implementation.

use proc_macro2::{Span, TokenStream};
use quote::quote;

/// Parse a safe equality condition (`field == value`) into the field name and a
/// `ferris_core::Value` expression.
///
/// # Errors
///
/// Returns an error when `expr` is not `field == value`, when the field is not a
/// bare identifier, or when the value is not a number, bool or quoted string.
///
/// The grammar is deliberately tiny — a bare identifier, `==`, then a number, a
/// double-quoted string or a bool — so a condition is *data*, never evaluated
/// code. `context` (`"workflow"`/`"permission"`) only shapes the error message.
///
/// A quoted value is parsed through [`syn`] rather than by slicing the outer
/// quotes off the raw string. Slicing left escape sequences half-processed: the
/// outer attribute already unescaped once, so `owner == "a\"b"` reached the
/// macro as the text `a\"b` and leaked a literal backslash into the value. Going
/// through [`syn::LitStr`] decodes the escapes exactly once.
pub(crate) fn parse_condition(
    expr: &str,
    context: &str,
    span: Span,
) -> Result<(String, TokenStream), syn::Error> {
    let (field, raw_value) = expr.split_once("==").ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "{context} condition must be a simple equality, e.g. `field == 0` or `field == \"value\"`"
            ),
        )
    })?;
    let field = field.trim();
    if field.is_empty() || !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(syn::Error::new(
            span,
            format!("invalid {context} condition field `{field}`"),
        ));
    }
    Ok((
        field.to_string(),
        condition_value(raw_value.trim(), context, span)?,
    ))
}

/// Turn the right-hand side of a condition into the `Value` constructor the
/// macro emits. Kept separate so the number/bool/string cases are testable
/// without a full attribute.
fn condition_value(raw: &str, context: &str, span: Span) -> Result<TokenStream, syn::Error> {
    if raw.starts_with('"') {
        // Parse through `syn` so escapes (`\"`, `\n`, …) are decoded exactly
        // once instead of leaking a backslash into the runtime value.
        let lit: syn::LitStr = syn::parse_str(raw).map_err(|_| {
            syn::Error::new(span, format!("invalid {context} condition string `{raw}`"))
        })?;
        let value = lit.value();
        return Ok(quote!(ferris_core::Value::from(#value)));
    }
    if raw == "true" || raw == "false" {
        let value = raw == "true";
        return Ok(quote!(ferris_core::Value::from(#value)));
    }
    if let Ok(value) = raw.parse::<i64>() {
        return Ok(quote!(ferris_core::Value::from(#value)));
    }
    if let Ok(value) = raw.parse::<f64>() {
        return Ok(quote!(ferris_core::Value::from(#value)));
    }
    Err(syn::Error::new(
        span,
        format!(
            "unsupported {context} condition value `{raw}`; use a number, a double-quoted string or a bool"
        ),
    ))
}

/// Map a Rust type name to the `snake_case` base of a generated function name
/// (`SalesOrder` → `sales_order`).
///
/// Deliberately simple: it lower-cases and inserts `_` before every uppercase
/// letter. The inputs are Rust type identifiers, so an acronym (`HTTPServer`)
/// becomes `h_t_t_p_server` — ugly, but stable and collision-free, which is what
/// a generated symbol needs. A "smarter" splitter would rename every existing
/// generated function, breaking callers pinned to the current name, for no
/// functional gain.
#[must_use]
pub(crate) fn to_snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests;
