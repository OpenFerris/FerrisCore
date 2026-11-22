//! `validate()` code generation for `#[doctype]`.
//!
//! Split out of the expansion module: the generated body is the only place that
//! knows which field types have a runtime check, and it is worth reading on its
//! own.

use crate::field::{FieldDef, FieldType};
use proc_macro2::TokenStream;
use quote::quote;

/// Emit the body of the generated `Document::validate` (everything between the
/// metadata warm-up and the final `Ok(())`).
///
/// The struct is serialised to JSON **once** into a local named
/// `__ferris_fields` and every check reads from that map. The previous version
/// called `self.get_field(name)` per field, and each of those re-serialised the
/// whole document, so validating an `n`-field `DocType` cost `2n` serialisations
///.
pub(super) fn generate_validation(fields: &[FieldDef]) -> TokenStream {
    // Required-field checks.
    let validations: Vec<TokenStream> = fields
        .iter()
        .filter(|f| f.reqd)
        .map(|f| {
            let field_name = &f.fieldname;
            let label = f.label.as_deref().unwrap_or(&f.fieldname);
            quote! {
                if __ferris_fields.get(#field_name).map_or(true, |value| {
                    ferris_core::Value::from_json(value.clone()).is_null()
                }) {
                    return Err(ferris_core::Error::Validation(
                        ferris_core::ValidationError::Required(#label.to_string())
                    ));
                }
            }
        })
        .collect();

    // Type validations.
    let type_validations: Vec<TokenStream> = fields
        .iter()
        .filter_map(|f| {
            let field_name = &f.fieldname;
            let label = f.label.as_deref().unwrap_or(&f.fieldname);

            match f.fieldtype {
                FieldType::Select => {
                    // `options` is one value per line — the same contract
                    // `ferris_document::validation::validate_select` and the Desk
                    // (`field_types`, `kanban_view`, `filter_group`) rely on.
                    let options = f.options.as_deref().unwrap_or("");
                    let valid_values: Vec<String> = options
                        .split('\n')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    Some(quote! {
                        if let Some(value) = __ferris_fields.get(#field_name) {
                            let value = ferris_core::Value::from_json(value.clone());
                            if let Some(text) = value.as_str() {
                                let valid = [#(#valid_values),*];
                                if !valid.contains(&text) {
                                    return Err(ferris_core::Error::Validation(
                                        ferris_core::ValidationError::Field {
                                            field: #label.to_string(),
                                            message: format!("Invalid {}: {}", #label, text)
                                        }
                                    ));
                                }
                            }
                        }
                    })
                }
                FieldType::Email => Some(string_check(f.fieldname.as_str(), label, "email")),
                FieldType::Phone => Some(string_check(f.fieldname.as_str(), label, "phone")),
                FieldType::URL => Some(string_check(f.fieldname.as_str(), label, "url")),
                _ => None,
            }
        })
        .collect();

    quote! {
        // Serialise once; the borrowed map below is read by every check.
        let __ferris_fields = ferris_document::serde_json::to_value(self)
            .unwrap_or(ferris_document::serde_json::Value::Null);
        #(#validations)*
        #(#type_validations)*
    }
}

/// One `is_valid_<kind>` string check against a field.
fn string_check(field_name: &str, label: &str, kind: &str) -> TokenStream {
    let predicate = match kind {
        "email" => quote! { ferris_document::is_valid_email(text) },
        "phone" => quote! { ferris_document::is_valid_phone(text) },
        _ => quote! { ferris_document::is_valid_url(text) },
    };
    let message = format!("Invalid {kind} format");
    quote! {
        if let Some(value) = __ferris_fields.get(#field_name) {
            let value = ferris_core::Value::from_json(value.clone());
            if let Some(text) = value.as_str() {
                if !#predicate {
                    return Err(ferris_core::Error::Validation(
                        ferris_core::ValidationError::Field {
                            field: #label.to_string(),
                            message: #message.to_string()
                        }
                    ));
                }
            }
        }
    }
}
