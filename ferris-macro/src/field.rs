//! `#[field(...)]` parsing and the Rust-type → field-type inference.
//!
//! `FieldType` lives in [`field_type`]; this module turns one `syn::Field` into
//! a [`FieldDef`] and knows how to emit the matching `ferris_schema::FieldMeta`
//! struct literal.

use proc_macro2::Span;
use quote::ToTokens;
use syn::{Field, LitStr};

mod field_type;

pub use field_type::FieldType;

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub fieldname: String,
    pub label: Option<String>,
    pub fieldtype: FieldType,
    pub options: Option<String>,
    pub default: Option<String>,
    pub reqd: bool,
    pub read_only: bool,
    pub hidden: bool,
    pub depends_on: Option<String>,
    pub description: Option<String>,
    pub in_list_view: bool,
    pub in_standard_filter: bool,
    pub in_global_search: bool,
    pub in_preview: bool,
    pub bold: bool,
    pub print_hide: bool,
    pub report_hide: bool,
    pub permlevel: u8,
    pub unique: bool,
    pub search_index: bool,
    pub sort_options: bool,
    pub allow_on_submit: bool,
    pub no_copy: bool,
    pub ignore_user_permissions: bool,
    pub ignore_xss_filter: bool,
    pub convert_to_user_timezone: bool,
    pub columns: u8,
    pub fetch_from: Option<String>,
    pub fetch_if_empty: bool,
    pub print_width: Option<String>,
    pub width: Option<String>,
    pub placeholder: Option<String>,
    pub max_length: Option<u32>,
    pub max_value: Option<String>,
    pub min_value: Option<String>,
    pub precision: Option<u8>,
    pub length: Option<u32>,
    pub read_only_on_load: bool,
}

impl FieldDef {
    pub fn from_field(field: &Field) -> syn::Result<Self> {
        let fieldname = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new(Span::call_site(), "Field must have a name"))?
            .to_string();

        let mut def = FieldDef {
            fieldname,
            label: None,
            fieldtype: FieldType::Data,
            options: None,
            default: None,
            reqd: false,
            read_only: false,
            hidden: false,
            depends_on: None,
            description: None,
            in_list_view: false,
            in_standard_filter: false,
            in_global_search: false,
            in_preview: false,
            bold: false,
            print_hide: false,
            report_hide: false,
            permlevel: 0,
            unique: false,
            search_index: false,
            sort_options: false,
            allow_on_submit: false,
            no_copy: false,
            ignore_user_permissions: false,
            ignore_xss_filter: false,
            convert_to_user_timezone: false,
            columns: 0,
            fetch_from: None,
            fetch_if_empty: false,
            print_width: None,
            width: None,
            placeholder: None,
            max_length: None,
            max_value: None,
            min_value: None,
            precision: None,
            length: None,
            read_only_on_load: false,
        };

        // Parse attributes
        for attr in &field.attrs {
            if attr.path().is_ident("field") {
                attr.parse_nested_meta(|meta| {
                    let ident = meta
                        .path
                        .get_ident()
                        .ok_or_else(|| meta.error("expected identifier"))?
                        .to_string();

                    match ident.as_str() {
                        "type" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            let name = lit.value();
                            // Validate here: an unknown type must be a compile
                            // error, not a string no consumer understands.
                            def.fieldtype = FieldType::from_name(&name).ok_or_else(|| {
                                meta.error(format!("unknown field type `{name}`"))
                            })?;
                        }
                        "label" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.label = Some(lit.value());
                        }
                        "options" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.options = Some(lit.value());
                        }
                        "default" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.default = Some(lit.value());
                        }
                        "reqd" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.reqd = lit.value();
                        }
                        "read_only" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.read_only = lit.value();
                        }
                        "hidden" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.hidden = lit.value();
                        }
                        "depends_on" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.depends_on = Some(lit.value());
                        }
                        "description" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.description = Some(lit.value());
                        }
                        "in_list_view" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.in_list_view = lit.value();
                        }
                        "in_standard_filter" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.in_standard_filter = lit.value();
                        }
                        "in_global_search" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.in_global_search = lit.value();
                        }
                        "in_preview" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.in_preview = lit.value();
                        }
                        "bold" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.bold = lit.value();
                        }
                        "print_hide" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.print_hide = lit.value();
                        }
                        "report_hide" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.report_hide = lit.value();
                        }
                        "permlevel" => {
                            let value = meta.value()?;
                            let lit: syn::LitInt = value.parse()?;
                            def.permlevel = lit.base10_parse()?;
                        }
                        "unique" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.unique = lit.value();
                        }
                        "search_index" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.search_index = lit.value();
                        }
                        "sort_options" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.sort_options = lit.value();
                        }
                        "allow_on_submit" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.allow_on_submit = lit.value();
                        }
                        "no_copy" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.no_copy = lit.value();
                        }
                        "ignore_user_permissions" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.ignore_user_permissions = lit.value();
                        }
                        "ignore_xss_filter" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.ignore_xss_filter = lit.value();
                        }
                        "convert_to_user_timezone" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.convert_to_user_timezone = lit.value();
                        }
                        "columns" => {
                            let value = meta.value()?;
                            let lit: syn::LitInt = value.parse()?;
                            def.columns = lit.base10_parse()?;
                        }
                        "fetch_from" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.fetch_from = Some(lit.value());
                        }
                        "fetch_if_empty" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.fetch_if_empty = lit.value();
                        }
                        "print_width" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.print_width = Some(lit.value());
                        }
                        "width" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.width = Some(lit.value());
                        }
                        "placeholder" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.placeholder = Some(lit.value());
                        }
                        "max_length" => {
                            let value = meta.value()?;
                            let lit: syn::LitInt = value.parse()?;
                            def.max_length = Some(lit.base10_parse()?);
                        }
                        "max_value" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.max_value = Some(lit.value());
                        }
                        "min_value" => {
                            let value = meta.value()?;
                            let lit: LitStr = value.parse()?;
                            def.min_value = Some(lit.value());
                        }
                        "precision" => {
                            let value = meta.value()?;
                            let lit: syn::LitInt = value.parse()?;
                            def.precision = Some(lit.base10_parse()?);
                        }
                        "length" => {
                            let value = meta.value()?;
                            let lit: syn::LitInt = value.parse()?;
                            def.length = Some(lit.base10_parse()?);
                        }
                        "read_only_on_load" => {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            def.read_only_on_load = lit.value();
                        }
                        _ => return Err(meta.error(format!("Unknown field attribute: {}", ident))),
                    }
                    Ok(())
                })?;
            }
        }

        // Infer the field type from the Rust type when the attribute did not
        // name one explicitly.
        if def.fieldtype == FieldType::Data {
            def.fieldtype = Self::infer_field_type(&field.ty);
        }

        Ok(def)
    }

    /// Infer a field type from the Rust type of the struct field.
    ///
    /// Structural, not textual: the previous version stringified the type and
    /// ran `contains` on the result, so `Option<MyBool>` matched `bool` and
    /// became a `Check`, and `Vec` was only recognised outside an `Option`
    ///. Here an `Option<T>` is unwrapped first and only the last
    /// path segment is inspected, so a wrapper never masquerades as its name.
    fn infer_field_type(ty: &syn::Type) -> FieldType {
        let inner = Self::option_inner(ty).unwrap_or(ty);
        let syn::Type::Path(path) = inner else {
            return FieldType::Data;
        };
        let Some(segment) = path.path.segments.last() else {
            return FieldType::Data;
        };
        Self::from_rust_type_name(&segment.ident.to_string())
    }

    /// If `ty` is `Option<T>`, return the `T`.
    fn option_inner(ty: &syn::Type) -> Option<&syn::Type> {
        let syn::Type::Path(path) = ty else {
            return None;
        };
        let segment = path.path.segments.last()?;
        if segment.ident != "Option" {
            return None;
        }
        let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
            return None;
        };
        args.args.iter().find_map(|arg| match arg {
            syn::GenericArgument::Type(inner) => Some(inner),
            _ => None,
        })
    }

    /// The Rust type names that map to a non-`Data` field type. Matched exactly
    /// against the last path segment.
    fn from_rust_type_name(name: &str) -> FieldType {
        match name {
            "bool" | "Boolean" => FieldType::Check,
            "NaiveDate" => FieldType::Date,
            "DateTime" | "NaiveDateTime" => FieldType::Datetime,
            "NaiveTime" => FieldType::Time,
            "Decimal" => FieldType::Currency,
            "f32" | "f64" => FieldType::Float,
            "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64"
            | "u128" | "usize" => FieldType::Int,
            "Vec" => FieldType::Table,
            "Value" | "Json" | "JsonValue" => FieldType::JSON,
            "Link" => FieldType::Link,
            _ => FieldType::Data,
        }
    }
}

impl ToTokens for FieldDef {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let fieldname = &self.fieldname;
        let fieldtype = self.fieldtype.as_str();

        let opt_str = |value: &Option<String>| match value {
            Some(v) => quote::quote! { Some(#v.to_string()) },
            None => quote::quote! { None },
        };
        let opt_u32 = |value: &Option<u32>| match value {
            Some(v) => quote::quote! { Some(#v) },
            None => quote::quote! { None },
        };
        let opt_u8 = |value: &Option<u8>| match value {
            Some(v) => quote::quote! { Some(#v) },
            None => quote::quote! { None },
        };

        let label = opt_str(&self.label);
        let options = opt_str(&self.options);
        let default = opt_str(&self.default);
        let depends_on = opt_str(&self.depends_on);
        let description = opt_str(&self.description);
        let fetch_from = opt_str(&self.fetch_from);
        let print_width = opt_str(&self.print_width);
        let width = opt_str(&self.width);
        let placeholder = opt_str(&self.placeholder);
        let max_value = opt_str(&self.max_value);
        let min_value = opt_str(&self.min_value);
        let max_length = opt_u32(&self.max_length);
        let length = opt_u32(&self.length);
        let precision = opt_u8(&self.precision);

        let reqd = self.reqd;
        let read_only = self.read_only;
        let hidden = self.hidden;
        let in_list_view = self.in_list_view;
        let in_standard_filter = self.in_standard_filter;
        let in_global_search = self.in_global_search;
        let in_preview = self.in_preview;
        let bold = self.bold;
        let print_hide = self.print_hide;
        let report_hide = self.report_hide;
        let permlevel = self.permlevel;
        let unique = self.unique;
        let search_index = self.search_index;
        let sort_options = self.sort_options;
        let allow_on_submit = self.allow_on_submit;
        let no_copy = self.no_copy;
        let ignore_user_permissions = self.ignore_user_permissions;
        let ignore_xss_filter = self.ignore_xss_filter;
        let convert_to_user_timezone = self.convert_to_user_timezone;
        let columns = self.columns;
        let fetch_if_empty = self.fetch_if_empty;
        let read_only_on_load = self.read_only_on_load;

        quote::quote! {
            ferris_schema::FieldMeta {
                fieldname: #fieldname.to_string(),
                label: #label,
                fieldtype: #fieldtype.to_string(),
                options: #options,
                default: #default,
                reqd: #reqd,
                read_only: #read_only,
                hidden: #hidden,
                depends_on: #depends_on,
                description: #description,
                in_list_view: #in_list_view,
                in_standard_filter: #in_standard_filter,
                in_global_search: #in_global_search,
                in_preview: #in_preview,
                bold: #bold,
                print_hide: #print_hide,
                report_hide: #report_hide,
                permlevel: #permlevel,
                unique: #unique,
                search_index: #search_index,
                sort_options: #sort_options,
                allow_on_submit: #allow_on_submit,
                no_copy: #no_copy,
                ignore_user_permissions: #ignore_user_permissions,
                ignore_xss_filter: #ignore_xss_filter,
                convert_to_user_timezone: #convert_to_user_timezone,
                columns: #columns,
                fetch_from: #fetch_from,
                fetch_if_empty: #fetch_if_empty,
                print_width: #print_width,
                width: #width,
                placeholder: #placeholder,
                max_length: #max_length,
                max_value: #max_value,
                min_value: #min_value,
                precision: #precision,
                length: #length,
                read_only_on_load: #read_only_on_load,
                r#virtual: false,
                // The DocField property set grows (Frappe adds properties over
                // time); defaulting the ones the attribute grammar does not
                // expose keeps every existing `#[doctype]` compiling instead of
                // forcing a macro edit per property.
                ..Default::default()
            }
        }
        .to_tokens(tokens);
    }
}

#[cfg(test)]
mod tests;
