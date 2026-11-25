//! Document validation.
//!
//! One dispatch ([`validate_field`]) maps every canonical fieldtype to its
//! rule; the rules compose through the shared `number_value`/`integer_value`
//! parsers instead of copying them, so "what counts as a valid value of type
//! X" is stated once per type. Types without a rule (`Text`, `Heading`,
//! the layout breaks…) carry no invariant beyond `reqd`, `max_length` and
//! the declared bounds, which the same function applies to every field.

use ferris_core::{Result, Value};

/// Validate a single field
pub fn validate_field(
    field_name: &str,
    value: &Value,
    field_meta: &ferris_schema::registry::FieldMeta,
) -> Result<()> {
    // Required check. The Desk sends `""` for a cleared input, not `null`, so
    // testing only `is_null` let an empty required field through.
    if field_meta.reqd && (value.is_null() || value.as_str().is_some_and(str::is_empty)) {
        return Err(ferris_core::Error::Validation(
            ferris_core::ValidationError::Required(
                field_meta
                    .label
                    .as_ref()
                    .unwrap_or(&field_meta.fieldname)
                    .to_string(),
            ),
        ));
    }

    // Skip further validation if value is null and not required
    if value.is_null() && !field_meta.reqd {
        return Ok(());
    }

    // Type-specific validation
    match field_meta.fieldtype.as_str() {
        "Link" => validate_link(field_name, value, field_meta)?,
        "Select" => validate_select(field_name, value, field_meta)?,
        "Email" => validate_email(field_name, value)?,
        "Phone" => validate_phone(field_name, value)?,
        "URL" => validate_url(field_name, value)?,
        "Currency" | "Float" => validate_number(field_name, value, field_meta)?,
        "Int" => validate_integer(field_name, value, field_meta)?,
        // the typed fieldtypes that used to fall through the
        // catch-all and accepted anything (a `Percent` stored `"abc"`, a
        // `Signature` stored an object).
        "Percent" => validate_percent(field_name, value, field_meta)?,
        "Rating" => validate_rating(field_name, value, field_meta)?,
        "Duration" => validate_duration(field_name, value, field_meta)?,
        "Geolocation" => validate_geolocation(field_name, value)?,
        "Signature" => validate_signature(field_name, value)?,
        "Attach" | "Attach Image" => validate_attach(field_name, value)?,
        "Date" | "Datetime" | "Time" => {
            validate_datetime(field_name, value, &field_meta.fieldtype)?;
        }
        "Check" => validate_boolean(field_name, value)?,
        "JSON" => validate_json(field_name, value)?,
        _ => {}
    }

    // Length validation
    if let Some(max_len) = field_meta.max_length
        && let Some(s) = value.as_str()
        && s.chars().count() > max_len as usize
    {
        // A7: `s.len()` counts *bytes*, so `max_length: 10` rejected a
        // 10-character multibyte string (`"日本語日本語"` is 30 bytes). Frappe
        // compares characters (`len` on a unicode string), so do the same.
        return Err(ferris_core::Error::Validation(
            ferris_core::ValidationError::Field {
                field: field_name.to_string(),
                message: format!("Maximum length is {max_len} characters"),
            },
        ));
    }

    // Range validation for numbers
    if let Some(max_val) = &field_meta.max_value {
        validate_max_value(field_name, value, max_val)?;
    }

    if let Some(min_val) = &field_meta.min_value {
        validate_min_value(field_name, value, min_val)?;
    }

    Ok(())
}

fn field_error(field_name: &str, message: impl Into<String>) -> ferris_core::Error {
    ferris_core::Error::Validation(ferris_core::ValidationError::Field {
        field: field_name.to_string(),
        message: message.into(),
    })
}

use rules::*;

mod format;
mod rules;

#[cfg(test)]
mod tests;

pub use format::{is_valid_email, is_valid_phone, is_valid_url};
