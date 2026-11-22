use super::{FieldDef, FieldType};

fn ty(src: &str) -> syn::Type {
    syn::parse_str(src).expect("valid type")
}

/// (ampliado en , F1): the field type is validated at parse
/// time and `as_str` emits the canonical Frappe spelling for every known type,
/// which is the exact inverse of `from_name` on the canonical list.
#[test]
fn known_field_types_round_trip_to_the_canonical_name() {
    for name in [
        "Data",
        "Link",
        "Select",
        "Table",
        "Table MultiSelect",
        "Currency",
        "Float",
        "Int",
        "Percent",
        "Check",
        "Date",
        "Datetime",
        "Time",
        "Duration",
        "Week",
        "Text",
        "Small Text",
        "Long Text",
        "Text Editor",
        "Code",
        "Markdown Editor",
        "HTML",
        "HTML Editor",
        "Color",
        "Image",
        "File",
        "Attach",
        "Attach Image",
        "Attachment Gallery",
        "Signature",
        "Barcode",
        "Geolocation",
        "Phone",
        "Email",
        "URL",
        "Password",
        "Secret",
        "Rating",
        "Dynamic Link",
        "JSON",
        "Read Only",
        "Button",
        "Heading",
        "Tab Break",
        "Section Break",
        "Column Break",
    ] {
        let parsed = FieldType::from_name(name).expect("known field type");
        assert_eq!(parsed.as_str(), name);
    }
}

#[test]
fn unknown_field_type_is_rejected() {
    assert_eq!(FieldType::from_name("Dat"), None);
    assert_eq!(FieldType::from_name("datetime"), None);
    assert_eq!(FieldType::from_name(""), None);
    assert_eq!(FieldType::from_name("Long Int"), None);
    assert_eq!(FieldType::from_name("Icon"), None);
}

/// inference reads the Rust type's structure, not its spelling.
#[test]
fn inference_reads_the_rust_type_structure() {
    let cases = [
        ("bool", FieldType::Check),
        ("Option<bool>", FieldType::Check),
        ("String", FieldType::Data),
        ("Option<String>", FieldType::Data),
        ("Vec<u8>", FieldType::Table),
        ("Option<Vec<u8>>", FieldType::Table),
        ("Option<Link<ToDo>>", FieldType::Link),
        ("Option<Value>", FieldType::JSON),
        ("Option<f64>", FieldType::Float),
        ("Option<NaiveDate>", FieldType::Date),
        ("Option<chrono::DateTime<Utc>>", FieldType::Datetime),
        ("ferris_core::DocStatus", FieldType::Data),
    ];
    for (src, expected) in cases {
        assert_eq!(
            FieldDef::infer_field_type(&ty(src)),
            expected,
            "inference for `{src}`"
        );
    }
}

/// the old textual inference used `contains`, so a type whose *name*
/// merely contained `bool` was typed as a `Check`, and `Vec` nested in an
/// `Option` was missed.
#[test]
fn inference_is_not_fooled_by_a_name_that_contains_a_known_one() {
    assert_eq!(
        FieldDef::infer_field_type(&ty("Option<MyBool>")),
        FieldType::Data
    );
    assert_eq!(
        FieldDef::infer_field_type(&ty("Option<MyValue>")),
        FieldType::Data
    );
}
