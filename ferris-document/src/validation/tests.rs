use super::*;

#[test]
fn test_is_valid_email() {
    assert!(is_valid_email("test@example.com"));
    assert!(!is_valid_email("invalid"));
    assert!(!is_valid_email("@example.com"));
    // Private installs use dotless domains; the rest of the shape is checked.
    assert!(is_valid_email("admin@local"));
    assert!(!is_valid_email("a@.local"));
    assert!(!is_valid_email("a@local."));
}

#[test]
fn test_is_valid_phone() {
    assert!(is_valid_phone("+1-555-123-4567"));
    assert!(!is_valid_phone("123"));
}

/// DOC12: `is_valid_phone` used to strip every non-digit, so letters and other
/// garbage were ignored as long as ten digits remained.
#[test]
fn test_phone_rejects_letters_and_short_digit_runs() {
    assert!(!is_valid_phone("abcdefghij0123456789"));
    assert!(!is_valid_phone("call me on 555 123 4567 maybe"));
    assert!(!is_valid_phone("12345"));
    // Formats that are genuinely phone numbers still pass.
    assert!(is_valid_phone("+34 600 123 456"));
    assert!(is_valid_phone("(555) 123-4567"));
}

#[test]
fn test_is_valid_url() {
    assert!(is_valid_url("https://example.com"));
    assert!(!is_valid_url("not-a-url"));
}

fn meta(fieldtype: &str) -> ferris_schema::registry::FieldMeta {
    ferris_schema::registry::FieldMeta {
        fieldname: "amount".to_string(),
        fieldtype: fieldtype.to_string(),
        ..Default::default()
    }
}

/// A7: `min_value` was a no-op for `Float`/`Currency`, so a field with
/// `min_value: "0"` accepted -999999.
#[test]
fn test_number_min_value_is_enforced() {
    let mut m = meta("Float");
    m.min_value = Some("0".to_string());
    let err = validate_field("amount", &Value::from(-999_999), &m).unwrap_err();
    assert!(err.to_string().contains(">= 0"), "{err}");
    validate_field("amount", &Value::from(0), &m).unwrap();
}

#[test]
fn test_number_max_value_is_enforced() {
    let mut m = meta("Currency");
    m.max_value = Some("1000".to_string());
    let err = validate_field("amount", &Value::from(1500), &m).unwrap_err();
    assert!(err.to_string().contains("<= 1000"), "{err}");
}

/// A7: `precision` was read nowhere, so `precision: 2` stored 1.239.
#[test]
fn test_currency_precision_is_enforced() {
    let mut m = meta("Currency");
    m.precision = Some(2);
    let err = validate_field("amount", &Value::from(1.239), &m).unwrap_err();
    assert!(err.to_string().contains("2 decimal places"), "{err}");
    validate_field("amount", &Value::from(1.24), &m).unwrap();
}

/// A7: Float/Currency values that parse as no number at all are refused.
#[test]
fn test_non_numeric_values_are_rejected_for_numbers() {
    let m = meta("Float");
    let err = validate_field("amount", &ferris_core::value!({ "a": 1 }), &m).unwrap_err();
    assert!(err.to_string().contains("must be a number"), "{err}");
    validate_field("amount", &Value::from("42.5"), &m).unwrap();
}

/// A7: a `Date` field used to accept `12345`, `{"a":1}` and garbage text.
#[test]
fn test_date_field_rejects_non_dates() {
    let m = meta("Date");
    let err = validate_field("when", &Value::from(12345), &m).unwrap_err();
    assert!(err.to_string().contains("must be a string"), "{err}");
    let err = validate_field("when", &ferris_core::value!({ "a": 1 }), &m).unwrap_err();
    assert!(err.to_string().contains("must be a string"), "{err}");
    let err = validate_field("when", &Value::from("no-es-una-fecha"), &m).unwrap_err();
    assert!(err.to_string().contains("Invalid Date"), "{err}");
    validate_field("when", &Value::from("2026-10-08"), &m).unwrap();
}

#[test]
fn test_datetime_field_accepts_iso8601() {
    let m = meta("Datetime");
    validate_field("when", &Value::from("2026-10-08T10:30:00Z"), &m).unwrap();
    validate_field("when", &Value::from("2026-10-08 10:30:00"), &m).unwrap();
    let err = validate_field("when", &Value::from("2026/10/08"), &m).unwrap_err();
    assert!(err.to_string().contains("Invalid Datetime"), "{err}");
}

/// A7: `max_length` compared `len()` (bytes), rejecting multibyte strings
/// that fit — 10 characters of 日本語 occupy 30 bytes.
#[test]
fn test_max_length_counts_characters_not_bytes() {
    let mut m = meta("Data");
    m.fieldname = "title".to_string();
    m.max_length = Some(10);
    // 6 characters / 18 bytes at max_length 10 — byte counting would
    // reject it, character counting admits it.
    validate_field("title", &Value::from("日本語日本語"), &m).unwrap();
    // 12 characters / 36 bytes — over the limit either way.
    let err = validate_field("title", &Value::from("日本語日本語日本語日本語"), &m).unwrap_err();
    assert!(
        err.to_string().contains("Maximum length is 10 characters"),
        "{err}"
    );
}

/// DOC3: an `Int` field used to accept anything `Value::as_i64` could not read
/// (`"abc"`, `true`, `1.5`, `[]`, `{}`) because the range check was the whole
/// body and was entered only for an `i64`. Every one of those is refused now,
/// while a real integer (and a numeric string) still passes.
#[test]
fn test_int_field_rejects_non_integers() {
    let m = meta("Int");
    for bad in [
        Value::from("abc"),
        Value::from(true),
        Value::from(1.5),
        Value::from(Vec::<Value>::new()),
        ferris_core::value!({ "a": 1 }),
    ] {
        let err = validate_field("amount", &bad, &m).unwrap_err();
        assert!(
            err.to_string().contains("must be an integer"),
            "{bad:?} -> {err}"
        );
    }
    validate_field("amount", &Value::from(7), &m).unwrap();
    validate_field("amount", &Value::from("42"), &m).unwrap();
}

/// DOC3: the min/max range still applies to an `Int`, and the value that fails
/// the range is an integer (not a shape failure).
#[test]
fn test_int_field_still_enforces_range() {
    let mut m = meta("Int");
    m.min_value = Some("1".to_string());
    m.max_value = Some("10".to_string());
    let err = validate_field("amount", &Value::from(0), &m).unwrap_err();
    assert!(err.to_string().contains(">= 1"), "{err}");
    let err = validate_field("amount", &Value::from(11), &m).unwrap_err();
    assert!(err.to_string().contains("<= 10"), "{err}");
    validate_field("amount", &Value::from(5), &m).unwrap();
}

// ---- the typed fieldtypes that used to fall through the catch-all ----

/// `Percent` is a number whose only bounds are the metadata's: -5% refunds
/// and 150% bonuses are legal values Frappe stores without complaint.
#[test]
fn percent_is_a_number_with_metadata_as_the_only_bound() {
    let m = meta("Percent");
    validate_field("amount", &Value::from(0), &m).unwrap();
    validate_field("amount", &Value::from(-5), &m).unwrap();
    validate_field("amount", &Value::from(150), &m).unwrap();
    validate_field("amount", &Value::from("87.5"), &m).unwrap();
    let err = validate_field("amount", &Value::from("no-es-un-numero"), &m).unwrap_err();
    assert!(err.to_string().contains("must be a number"), "{err}");

    let mut ranged = meta("Percent");
    ranged.min_value = Some("0".to_string());
    let err = validate_field("amount", &Value::from(-1), &ranged).unwrap_err();
    assert!(err.to_string().contains(">= 0"), "{err}");
}

/// `Rating` defaults to the 0..=5 star range, and a declared `max_value`
/// *replaces* the default instead of narrowing it.
#[test]
fn rating_defaults_to_the_star_range_unless_metadata_says_otherwise() {
    let m = meta("Rating");
    validate_field("amount", &Value::from(0), &m).unwrap();
    validate_field("amount", &Value::from(4.5), &m).unwrap();
    validate_field("amount", &Value::from("5"), &m).unwrap();
    let err = validate_field("amount", &Value::from(5.5), &m).unwrap_err();
    assert!(err.to_string().contains("<= 5"), "{err}");
    let err = validate_field("amount", &Value::from(-1), &m).unwrap_err();
    assert!(err.to_string().contains(">= 0"), "{err}");

    let mut widened = meta("Rating");
    widened.max_value = Some("10".to_string());
    validate_field("amount", &Value::from(7), &widened).unwrap();
}

/// `Duration` stores whole elapsed seconds: an integer, non-negative by
/// default.
#[test]
fn duration_is_whole_non_negative_seconds() {
    let m = meta("Duration");
    validate_field("amount", &Value::from(0), &m).unwrap();
    validate_field("amount", &Value::from(3661), &m).unwrap();
    validate_field("amount", &Value::from("90"), &m).unwrap();
    let err = validate_field("amount", &Value::from(-5), &m).unwrap_err();
    assert!(err.to_string().contains(">= 0"), "{err}");
    let err = validate_field("amount", &Value::from(1.5), &m).unwrap_err();
    assert!(err.to_string().contains("must be an integer"), "{err}");
    let err = validate_field("amount", &Value::from("abc"), &m).unwrap_err();
    assert!(err.to_string().contains("must be an integer"), "{err}");
}

/// `Geolocation` accepts the three shapes the Desk can produce — GeoJSON
/// text, the parsed object and the legacy `"lat,lon"` pair — plus the
/// cleared field.
#[test]
fn geolocation_accepts_geojson_and_the_legacy_latlon_pair() {
    let m = meta("Geolocation");
    validate_field(
        "amount",
        &Value::from(r#"{"type":"Point","coordinates":[-3.7038,40.4168]}"#),
        &m,
    )
    .unwrap();
    validate_field(
        "amount",
        &Value::from(r#"{"type":"FeatureCollection","features":[]}"#),
        &m,
    )
    .unwrap();
    validate_field("amount", &Value::from("40.4168, -3.7038"), &m).unwrap();
    let point = serde_json::from_str::<Value>(r#"{"type":"Point","coordinates":[0,0]}"#).unwrap();
    validate_field("amount", &point, &m).unwrap();
    validate_field("amount", &Value::from(""), &m).unwrap();
    // The empty Feature Frappe leaves behind: a null geometry is legal.
    validate_field(
        "amount",
        &Value::from(r#"{"type":"Feature","geometry":null}"#),
        &m,
    )
    .unwrap();
}

/// Anything that is not a location is refused at the store, even though the
/// map widget degrades gracefully on the client.
#[test]
fn geolocation_refuses_shapes_that_are_not_a_location() {
    let m = meta("Geolocation");
    for bad in [
        "91,0",  // latitude out of range
        "0,181", // longitude out of range
        "solo texto",
        r#"{"type":"Blob","coordinates":[0,0]}"#, // unknown type
        r#"{"type":"Point"}"#,                    // missing coordinates
        r#"{"type":"FeatureCollection"}"#,        // missing features
        r#"{"type":"Feature","geometry":{"type":"Feature"}}"#, // geometry must be a geometry
    ] {
        let err = validate_field("amount", &Value::from(bad), &m).unwrap_err();
        assert!(
            err.to_string().contains("Geolocation") || err.to_string().contains("GeoJSON"),
            "{bad}: {err}"
        );
    }
    let err = validate_field("amount", &Value::from(42), &m).unwrap_err();
    assert!(err.to_string().contains("must be a GeoJSON"), "{err}");
}

/// `Signature` is the canvas' base64 data URL — the same shape Frappe
/// stores — or the empty string for "cleared".
#[test]
fn signature_is_the_canvas_data_url_or_nothing() {
    let m = meta("Signature");
    validate_field(
        "amount",
        &Value::from("data:image/png;base64,iVBORw0KGgo="),
        &m,
    )
    .unwrap();
    validate_field("amount", &Value::from(""), &m).unwrap();
    for bad in [
        "firma.png",              // not a data URL
        "data:image/png;base64,", // empty payload
        "data:image/png,raw",     // declared but not base64
    ] {
        let err = validate_field("amount", &Value::from(bad), &m).unwrap_err();
        assert!(err.to_string().contains("data URL"), "{bad}: {err}");
    }
    let err = validate_field("amount", &ferris_core::value!({ "png": true }), &m).unwrap_err();
    assert!(err.to_string().contains("data URL"), "{err}");
}

/// `Attach`/`Attach Image` store the file's served path as a string; any
/// other storage shape would never reach the gallery or the `<img>`.
#[test]
fn attach_stores_a_path_string() {
    for fieldtype in ["Attach", "Attach Image"] {
        let m = meta(fieldtype);
        validate_field("amount", &Value::from("/files/informe.pdf"), &m).unwrap();
        validate_field("amount", &Value::from(""), &m).unwrap();
        let err = validate_field("amount", &ferris_core::value!({ "url": "x" }), &m).unwrap_err();
        assert!(err.to_string().contains("file path string"), "{err}");
        let err = validate_field("amount", &Value::from(7), &m).unwrap_err();
        assert!(err.to_string().contains("file path string"), "{err}");
    }
}

#[test]
fn a_required_field_rejects_an_empty_string() {
    let mut field = meta("Data");
    field.reqd = true;
    assert!(validate_field("title", &Value::Null, &field).is_err());
    assert!(validate_field("title", &Value::string(""), &field).is_err());
    assert!(validate_field("title", &Value::string("x"), &field).is_ok());
}
