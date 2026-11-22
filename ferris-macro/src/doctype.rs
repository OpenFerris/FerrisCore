//! `#[doctype(...)]` code generation.
//!
//! The expansion declares the fragments that live in sibling files; the files
//! themselves hold one concern each:
//!
//! - [`validation`] — the generated `Document::validate` body;
//! - [`permissions`] — `permissions = [ … ]` → `ferris_core::PermissionRule`.

mod permissions;
mod validation;

use crate::args::DoctypeArgs;
use crate::field::{FieldDef, FieldType};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Fields;
use validation::generate_validation;

/// Expand `#[doctype(...)] struct X { … }`.
///
/// # Errors
///
/// Fails when the struct has no named fields, when no `name`/`#[key]` field is
/// present, or when a field attribute is invalid.
pub fn expand(args: DoctypeArgs, input: syn::ItemStruct) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let struct_name_str = struct_name.to_string();

    // Extract fields from the struct
    let fields = match &input.fields {
        Fields::Named(fields) => &fields.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &input,
                "Doctypes must have named fields",
            ));
        }
    };

    // Parse field attributes (invalid attributes are a hard error).
    let field_defs: Vec<FieldDef> = fields
        .iter()
        .map(FieldDef::from_field)
        .collect::<syn::Result<Vec<_>>>()?;

    // Field names present on the struct, used to generate getters/setters only
    // for the standard Frappe columns the DocType actually declares.
    let field_names: std::collections::HashSet<String> = fields
        .iter()
        .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
        .collect();
    let key_ident = fields
        .iter()
        .find(|f| f.attrs.iter().any(|a| a.path().is_ident("key")))
        .or_else(|| {
            fields
                .iter()
                .find(|f| f.ident.as_ref().map(|i| i == "name").unwrap_or(false))
        })
        .and_then(|f| f.ident.clone())
        .ok_or_else(|| {
            syn::Error::new_spanned(
                struct_name,
                "doctype requires a `name` field or a #[key] field",
            )
        })?;
    let has = |name: &str| field_names.contains(name);

    // Generate doctype metadata
    let doctype_name = args.name.unwrap_or_else(|| struct_name_str.clone());
    let module = args.module.unwrap_or_else(|| "core".to_string());
    let is_submittable = args.is_submittable;
    let is_tree = args.is_tree;
    let is_virtual = args.is_virtual;
    let is_single = args.is_single;
    let is_child_table = args.is_child_table;
    let naming = args.naming.unwrap_or_else(|| {
        format!(
            "{}-.#####",
            doctype_name
                .to_uppercase()
                .chars()
                .take(3)
                .collect::<String>()
        )
    });

    // Declared naming strategy. The string maps to the shared contract enum so
    // the typed path, the registry and the Desk all read one declaration.
    let naming_type_tokens = match args.naming_type.as_deref() {
        None => quote! { None },
        Some(raw) => {
            // Both `naming-series` and `naming_series` are accepted: the
            // attribute grammar allows a bare word or a quoted string, and the
            // wire spelling is the snake_case one.
            let spelling = raw.replace('-', "_");
            let variant = match spelling.as_str() {
                "series" => quote! { ferris_schema::contract::NamingType::Series },
                "naming_series" => quote! { ferris_schema::contract::NamingType::NamingSeries },
                "uuid" => quote! { ferris_schema::contract::NamingType::Uuid },
                "auto" | "autoincrement" | "auto_increment" => {
                    quote! { ferris_schema::contract::NamingType::AutoIncrement }
                }
                "field" | "fieldname" => quote! { ferris_schema::contract::NamingType::Field },
                "prompt" => quote! { ferris_schema::contract::NamingType::Prompt },
                "expression" => quote! { ferris_schema::contract::NamingType::Expression },
                "expression_old_style" => {
                    quote! { ferris_schema::contract::NamingType::ExpressionOldStyle }
                }
                "random" => quote! { ferris_schema::contract::NamingType::Random },
                "script" => quote! { ferris_schema::contract::NamingType::Script },
                other => {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "unknown naming_type `{other}`; expected one of \
                             series, naming_series, auto, field, prompt, expression, \
                             expression_old_style, random, script"
                        ),
                    ));
                }
            };
            quote! { Some(#variant) }
        }
    };
    if matches!(
        args.naming_type
            .as_deref()
            .map(|raw| raw.replace('-', "_"))
            .as_deref(),
        Some("field" | "fieldname")
    ) && args.naming_field.is_none()
    {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "naming_type = \"field\" requires naming_field = \"<fieldname>\"",
        ));
    }
    let naming_field_tokens = match args.naming_field.as_deref() {
        None => quote! { None },
        Some(field) => quote! { Some(#field) },
    };
    // The registry stores `Option<String>`; the trait const is
    // `Option<&'static str>`. Same declaration, two shapes.
    let naming_field_meta_tokens = match args.naming_field.as_deref() {
        None => quote! { None },
        Some(field) => quote! { Some(#field.to_string()) },
    };

    // `in_*` UI flags, emitted from the one list in `args::InFlags`.
    let in_flags = args.in_flags.meta_fields();

    // Extract all args fields needed in quote
    let description = args.description.clone();
    let workflow = args.workflow.clone();
    let track_changes = args.track_changes;
    let track_views = args.track_views;
    let allow_rename = args.allow_rename;
    let allow_copy = args.allow_copy;
    let allow_delete = args.allow_delete;
    let allow_print = args.allow_print;
    let allow_email = args.allow_email;
    let allow_attach = args.allow_attach;
    let allow_import = args.allow_import;
    let allow_export = args.allow_export;
    let allow_share = args.allow_share;
    let allow_submit = args.allow_submit;
    let allow_cancel = args.allow_cancel;
    let allow_amend = args.allow_amend;
    let custom = args.custom;
    let beta = args.beta;
    let hidden = args.hidden;
    let read_only = args.read_only;
    let quick_entry = args.quick_entry;
    let sort_field = args.sort_field.clone();
    let sort_order = args.sort_order.clone();
    let image_field = args.image_field.clone();
    let title_field = args.title_field.clone();
    let subject_field = args.subject_field.clone();
    let sender_field = args.sender_field.clone();
    let recipient_field = args.recipient_field.clone();
    let content_field = args.content_field.clone();
    let date_field = args.date_field.clone();
    let color_field = args.color_field.clone();
    let icon_field = args.icon_field.clone();

    // Generate permission constants
    let perm_rules = &args.permissions;
    let perm_consts = permissions::generate_permission_consts(perm_rules);

    // Generate validation code
    let validation = generate_validation(&field_defs);

    // Generate naming series function
    let naming_fn = quote! { #naming.to_string() };

    // Generate child table handling
    let child_tables = generate_child_table_handling(&field_defs);

    // Prepare search_fields for quote
    let search_fields: Vec<TokenStream> = args
        .search_fields
        .iter()
        .map(|s| {
            let s = s.clone();
            quote! { #s.to_string() }
        })
        .collect();

    // Prepare indexes for quote
    let index_metas: Vec<TokenStream> = args
        .indexes
        .iter()
        .map(|idx| {
            let name = &idx.name;
            let fields = &idx.fields;
            let unique = idx.unique;
            let index_type = idx.index_type as i32;
            quote! {
                ferris_schema::IndexMeta {
                    name: #name.to_string(),
                    fields: vec![#(#fields.to_string()),*],
                    unique: #unique,
                    index_type: #index_type,
                }
            }
        })
        .collect();

    // Standard-column accessors, generated only when the struct declares the
    // column (so typed DocTypes stay free to omit unused columns).
    let get_name = quote! { &self.#key_ident };
    let set_name = quote! { self.#key_ident = name; };
    let get_owner = if has("owner") {
        quote! { &self.owner }
    } else {
        quote! { "" }
    };
    let set_owner = if has("owner") {
        quote! { self.owner = owner; }
    } else {
        quote! {}
    };
    let get_creation = if has("creation") {
        quote! { &self.creation }
    } else {
        quote! { &ferris_document::chrono::DateTime::<ferris_document::chrono::Utc>::UNIX_EPOCH }
    };
    let set_modified = if has("modified") {
        quote! { self.modified = modified; }
    } else {
        quote! {}
    };
    let get_modified = if has("modified") {
        quote! { &self.modified }
    } else {
        quote! { &ferris_document::chrono::DateTime::<ferris_document::chrono::Utc>::UNIX_EPOCH }
    };
    let get_modified_by = if has("modified_by") {
        quote! { &self.modified_by }
    } else {
        quote! { "" }
    };
    let set_modified_by = if has("modified_by") {
        quote! { self.modified_by = modified_by; }
    } else {
        quote! {}
    };
    let get_docstatus = if has("docstatus") {
        quote! { self.docstatus }
    } else {
        quote! { ferris_core::DocStatus::Draft }
    };
    let set_docstatus = if has("docstatus") {
        quote! { self.docstatus = docstatus; }
    } else {
        quote! {}
    };
    let get_parent = if has("parent") {
        quote! { self.parent.as_deref() }
    } else {
        quote! { None }
    };
    let set_parent = if has("parent") {
        quote! { self.parent = parent; }
    } else {
        quote! {}
    };
    let get_parentfield = if has("parentfield") {
        quote! { self.parentfield.as_deref() }
    } else {
        quote! { None }
    };
    let set_parentfield = if has("parentfield") {
        quote! { self.parentfield = parentfield; }
    } else {
        quote! {}
    };
    let get_parenttype = if has("parenttype") {
        quote! { self.parenttype.as_deref() }
    } else {
        quote! { None }
    };
    let set_parenttype = if has("parenttype") {
        quote! { self.parenttype = parenttype; }
    } else {
        quote! {}
    };
    let get_idx = if has("idx") {
        quote! { self.idx }
    } else {
        quote! { 0 }
    };
    let set_idx = if has("idx") {
        quote! { self.idx = idx; }
    } else {
        quote! {}
    };

    // ChildDocument accessors (so `Document::Child` can be the type itself for
    // non child-table DocTypes).
    let child_get_parent = if has("parent") {
        quote! { self.parent.clone() }
    } else {
        quote! { None }
    };
    let child_set_parent = if has("parent") {
        quote! { self.parent = Some(parent); }
    } else {
        quote! {}
    };
    let child_get_parentfield = if has("parentfield") {
        quote! { self.parentfield.clone() }
    } else {
        quote! { None }
    };
    let child_set_parentfield = if has("parentfield") {
        quote! { self.parentfield = Some(parentfield); }
    } else {
        quote! {}
    };
    let child_get_parenttype = if has("parenttype") {
        quote! { self.parenttype.clone() }
    } else {
        quote! { None }
    };
    let child_set_parenttype = if has("parenttype") {
        quote! { self.parenttype = Some(parenttype); }
    } else {
        quote! {}
    };

    // `quote!` interpolates `Option<T>` by emitting nothing for `None`, so an
    // `Option<String>` must be expanded to a full `Option<..>` expression here.
    let opt_string = |value: &Option<String>| match value {
        Some(s) => quote! { Some(#s.to_string()) },
        None => quote! { None },
    };
    let description_tokens = opt_string(&description);
    let workflow_tokens = opt_string(&workflow);
    let workflow_const = match &workflow {
        Some(w) => quote! { Some(#w) },
        None => quote! { None },
    };
    let sort_field_tokens = opt_string(&sort_field);
    let sort_order_tokens = opt_string(&sort_order);
    let image_field_tokens = opt_string(&image_field);
    let title_field_tokens = opt_string(&title_field);
    let subject_field_tokens = opt_string(&subject_field);
    let sender_field_tokens = opt_string(&sender_field);
    let recipient_field_tokens = opt_string(&recipient_field);
    let content_field_tokens = opt_string(&content_field);
    let date_field_tokens = opt_string(&date_field);
    let color_field_tokens = opt_string(&color_field);
    let icon_field_tokens = opt_string(&icon_field);

    let meta_fn = format_ident!("__ferris_doctype_meta_{}", struct_name);

    // Strip the macro-only field attributes before re-emitting the struct.
    let mut cleaned_input = input.clone();
    if let Fields::Named(ref mut named) = cleaned_input.fields {
        for field in named.named.iter_mut() {
            field
                .attrs
                .retain(|attr| !(attr.path().is_ident("field") || attr.path().is_ident("key")));
        }
    }

    Ok(quote! {
        // Original struct, with `#[field]`/`#[key]` helper attributes removed.
        #cleaned_input

        // Compile-time DocType metadata, registered lazily on first use.
        #[doc(hidden)]
        #[allow(non_snake_case)]
        fn #meta_fn() -> &'static ferris_schema::DoctypeMeta {
            static META: std::sync::OnceLock<ferris_schema::DoctypeMeta> = std::sync::OnceLock::new();
            META.get_or_init(|| {
                let fields = vec![#(#field_defs),*];
                let permissions = vec![#(#perm_consts),*];
                let indexes = vec![#(#index_metas),*];
                let meta = ferris_schema::DoctypeMeta {
                    name: #doctype_name.to_string(),
                    module: #module.to_string(),
                    description: #description_tokens,
                    fields,
                    permissions,
                    naming_series: #naming.to_string(),
                    naming_type: #naming_type_tokens,
                    naming_field: #naming_field_meta_tokens,
                    workflow: #workflow_tokens,
                    is_submittable: #is_submittable,
                    is_tree: #is_tree,
                    is_virtual: #is_virtual,
                    is_single: #is_single,
                    is_child_table: #is_child_table,
                    track_changes: #track_changes,
                    track_views: #track_views,
                    allow_rename: #allow_rename,
                    allow_copy: #allow_copy,
                    allow_delete: #allow_delete,
                    allow_print: #allow_print,
                    allow_email: #allow_email,
                    allow_attach: #allow_attach,
                    allow_import: #allow_import,
                    allow_export: #allow_export,
                    allow_share: #allow_share,
                    allow_submit: #allow_submit,
                    allow_cancel: #allow_cancel,
                    allow_amend: #allow_amend,
                    indexes,
                    search_fields: vec![#(#search_fields),*],
                    sort_field: #sort_field_tokens,
                    sort_order: #sort_order_tokens,
                    image_field: #image_field_tokens,
                    title_field: #title_field_tokens,
                    subject_field: #subject_field_tokens,
                    sender_field: #sender_field_tokens,
                    recipient_field: #recipient_field_tokens,
                    content_field: #content_field_tokens,
                    date_field: #date_field_tokens,
                    color_field: #color_field_tokens,
                    icon_field: #icon_field_tokens,
                    // DocType views (`DocView`, `DashboardView`) are declared
                    // through `DoctypeMetaBuilder` — a plugin ships them in
                    // `describe()`, a built-in doctype with
                    // `.doc_view(...)`/`.dashboard_view(...)` — and are never
                    // part of this attribute: a `#[doctype]` struct opens in
                    // the built-in form unless its registration says otherwise.
                    view: ferris_schema::contract::DocView::default(),
                    dashboard: ferris_schema::contract::DashboardView::default(),
                    custom: #custom,
                    beta: #beta,
                    hidden: #hidden,
                    read_only: #read_only,
                    quick_entry: #quick_entry,
                    // Only the `in_*` flags the attribute actually enabled are
                    // written; `DoctypeMeta::default()` supplies `false` for the
                    // rest, so adding a flag to `args::InFlags` cannot leave a
                    // gap here.
                    #in_flags
                    ..ferris_schema::DoctypeMeta::default()
                };
                ferris_schema::registry::register_doctype(meta.clone());
                meta
            })
        }

        impl #struct_name {
            /// Register this DocType's metadata in the global schema registry.
            pub fn ferris_register_doctype() -> &'static ferris_schema::DoctypeMeta {
                #meta_fn()
            }
        }

        #[ferris_document::async_trait]
        impl ferris_document::Document for #struct_name {
            type Key = String;
            type Child = #struct_name;

            const DOCTYPE: &'static str = #doctype_name;
            const MODULE: &'static str = #module;
            const IS_SUBMITTABLE: bool = #is_submittable;
            const IS_TREE: bool = #is_tree;
            const IS_VIRTUAL: bool = #is_virtual;
            const IS_SINGLE: bool = #is_single;
            const IS_CHILD_TABLE: bool = #is_child_table;
            const NAMING_SERIES: &'static str = #naming;
            const NAMING_TYPE: Option<ferris_schema::contract::NamingType> = #naming_type_tokens;
            const NAMING_FIELD: Option<&'static str> = #naming_field_tokens;
            const WORKFLOW: Option<&'static str> = #workflow_const;

            fn permissions() -> &'static [ferris_core::PermissionRule] {
                static RULES: std::sync::OnceLock<Vec<ferris_core::PermissionRule>> = std::sync::OnceLock::new();
                RULES.get_or_init(|| vec![#(#perm_consts),*]).as_slice()
            }

            fn child_table_fields() -> &'static [&'static str] {
                &[#(#child_tables)*]
            }

            fn naming_series() -> String {
                let _ = #meta_fn();
                #naming_fn
            }

            fn validate(&self) -> ferris_core::Result<()> {
                let _ = #meta_fn();
                #validation
                Ok(())
            }

            fn get_field(&self, field: &str) -> Option<ferris_core::Value> {
                ferris_document::serde_json::to_value(self)
                    .ok()
                    .and_then(|value| value.get(field).cloned())
                    .map(ferris_core::Value::from_json)
            }

            async fn insert(&mut self, db: &ferris_document::Db) -> ferris_core::Result<String> {
                ferris_document::crud::insert(self, db).await
            }
            async fn save(&mut self, db: &ferris_document::Db) -> ferris_core::Result<()> {
                ferris_document::crud::save(self, db).await
            }
            async fn delete(&self, db: &ferris_document::Db) -> ferris_core::Result<()> {
                ferris_document::crud::delete(self, db).await
            }
            async fn submit(&mut self, db: &ferris_document::Db) -> ferris_core::Result<()> {
                ferris_document::crud::submit(self, db).await
            }
            async fn cancel(&mut self, db: &ferris_document::Db) -> ferris_core::Result<()> {
                ferris_document::crud::cancel(self, db).await
            }
            async fn amend(&self, db: &ferris_document::Db) -> ferris_core::Result<String> {
                ferris_document::crud::amend(self, db).await
            }

            fn get_key(&self) -> &Self::Key { #get_name }
            fn get_name(&self) -> &str { #get_name }
            fn get_doctype(&self) -> &str { #doctype_name }
            fn get_owner(&self) -> &str { #get_owner }
            fn get_creation(&self) -> &ferris_document::chrono::DateTime<ferris_document::chrono::Utc> { #get_creation }
            fn get_modified(&self) -> &ferris_document::chrono::DateTime<ferris_document::chrono::Utc> { #get_modified }
            fn get_modified_by(&self) -> &str { #get_modified_by }
            fn get_docstatus(&self) -> ferris_core::DocStatus { #get_docstatus }
            fn get_parent(&self) -> Option<&str> { #get_parent }
            fn get_parentfield(&self) -> Option<&str> { #get_parentfield }
            fn get_parenttype(&self) -> Option<&str> { #get_parenttype }
            fn get_idx(&self) -> i32 { #get_idx }

            fn set_name(&mut self, name: String) { #set_name }
            fn set_owner(&mut self, owner: String) { #set_owner }
            fn set_modified(&mut self, modified: ferris_document::chrono::DateTime<ferris_document::chrono::Utc>) { #set_modified }
            fn set_modified_by(&mut self, modified_by: String) { #set_modified_by }
            fn set_docstatus(&mut self, docstatus: ferris_core::DocStatus) { #set_docstatus }
            fn set_parent(&mut self, parent: Option<String>) { #set_parent }
            fn set_parentfield(&mut self, parentfield: Option<String>) { #set_parentfield }
            fn set_parenttype(&mut self, parenttype: Option<String>) { #set_parenttype }
            fn set_idx(&mut self, idx: i32) { #set_idx }
        }

        impl ferris_document::ChildDocument for #struct_name {
            fn get_parent(&self) -> Option<String> { #child_get_parent }
            fn set_parent(&mut self, parent: String) { #child_set_parent }
            fn get_parentfield(&self) -> Option<String> { #child_get_parentfield }
            fn set_parentfield(&mut self, parentfield: String) { #child_set_parentfield }
            fn get_parenttype(&self) -> Option<String> { #child_get_parenttype }
            fn set_parenttype(&mut self, parenttype: String) { #child_set_parenttype }
            fn get_idx(&self) -> i32 { #get_idx }
            fn set_idx(&mut self, idx: i32) { #set_idx }
        }

    })
}

/// The field names whose type is `Table`, in declaration order.
fn generate_child_table_handling(fields: &[FieldDef]) -> Vec<String> {
    fields
        .iter()
        .filter(|f| f.fieldtype == FieldType::Table)
        .map(|f| f.fieldname.clone())
        .collect()
}
