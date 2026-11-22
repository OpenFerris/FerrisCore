use ferris_core::{Error, Result, ValidationError};

pub fn validate_doctype(doctype: &str) -> Result<()> {
    if doctype.is_empty()
        || doctype.len() > 255
        || !doctype
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '_')
    {
        return Err(Error::Validation(ValidationError::Invalid(
            "invalid doctype".to_string(),
        )));
    }
    Ok(())
}

/// Validate a document name's character set and length.
///
/// M7: this used to reject only the empty string and `\0`, so a name
/// could embed `/`, backslashes, control characters or leading/trailing
/// whitespace — characters that are hostile in a URL path, a file name or a
/// LIKE pattern. Names are primary keys and, on `create`, come straight out of
/// the request body, so every character that reaches a route parameter is
/// adjudicated here. Frappe's rule is "no slashes"
/// (`frappe/model/base_document.py`), extended to the characters no document
/// name ever legitimately needs.
pub fn validate_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() != name.len() || name.len() > 255 || name.contains('\0')
    {
        return Err(Error::Validation(ValidationError::Invalid(
            "invalid document name".to_string(),
        )));
    }
    if name.contains('/') || name.contains('\\') || name.bytes().any(|b| b < 0x20) {
        return Err(Error::Validation(ValidationError::Invalid(
            "invalid document name: must not contain slashes, backslashes or control characters"
                .to_string(),
        )));
    }
    Ok(())
}
