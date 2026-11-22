use proc_macro2::Span;

use super::*;

#[test]
fn snake_case_maps_type_names_to_generated_symbols() {
    assert_eq!(to_snake_case("SalesOrder"), "sales_order");
    assert_eq!(to_snake_case("ToDo"), "to_do");
    assert_eq!(to_snake_case("Already_snake"), "already_snake");
    // Acronyms are split letter-by-letter on purpose; see the doc comment.
    assert_eq!(to_snake_case("HTTPServer"), "h_t_t_p_server");
    assert_eq!(to_snake_case(""), "");
}

fn value_tokens(expr: &str) -> String {
    parse_condition(expr, "workflow", Span::call_site())
        .expect("condition parses")
        .1
        .to_string()
}

#[test]
fn condition_keeps_the_field_name_out_of_the_value() {
    let (field, _) = parse_condition("grand_total == 0", "workflow", Span::call_site()).unwrap();
    assert_eq!(field, "grand_total");
}

#[test]
fn condition_accepts_numbers_bools_and_strings() {
    assert!(value_tokens("qty == 42").contains("42i64"));
    assert!(value_tokens("qty == -7").contains("7i64"));
    assert!(value_tokens("rate == 1.5").contains("1.5f64"));
    assert!(value_tokens("enabled == true").contains("true"));
    assert!(value_tokens("owner == \"alice\"").contains("alice"));
}

/// the value used to be produced by slicing the surrounding quotes
/// off the *already unescaped* attribute string, so an escaped quote leaked a
/// literal backslash into the runtime value.
#[test]
fn condition_decodes_string_escapes_exactly_once() {
    let tokens = value_tokens(r#"owner == "a\"b""#);
    // The emitted literal must decode to `a"b`, i.e. one escaped quote, not
    // `a\"b` (which is what the old slicing produced).
    assert!(tokens.contains(r#"a\"b"#), "unexpected tokens: {tokens}");
    assert!(!tokens.contains(r#"a\\\"b"#), "double-escaped: {tokens}");
}

#[test]
fn condition_rejects_non_equality_and_unknown_values() {
    assert!(parse_condition("grand_total", "workflow", Span::call_site()).is_err());
    assert!(parse_condition("== 0", "workflow", Span::call_site()).is_err());
    assert!(parse_condition("a.b == 0", "workflow", Span::call_site()).is_err());
    assert!(parse_condition("owner == alice", "workflow", Span::call_site()).is_err());
}
