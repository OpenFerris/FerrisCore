//! `#[doctype(...)]` arguments.
//!
//! The `in_*` UI flags are declared once in [`super::in_flags`]; everything else
//! is a plain field here.

use super::{InFlags, PermissionRule};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token};

/// `#[doctype(...)]` arguments.
///
/// This is one flat bag of knobs the attribute grammar exposes, so the
/// `clippy::struct_excessive_bools` heuristic does not apply: the booleans are
/// independent flags, not a state machine. The lint is allowed here, at the
/// definition, rather than at every use site.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default)]
pub(crate) struct DoctypeArgs {
    pub name: Option<String>,
    pub module: Option<String>,
    pub description: Option<String>,
    pub permissions: Vec<PermissionRule>,
    pub naming: Option<String>,
    /// Declared naming strategy (`series`, `uuid`, `auto`, `field`, `prompt`).
    pub naming_type: Option<String>,
    /// Field the name is read from when `naming_type = "field"`.
    pub naming_field: Option<String>,
    pub workflow: Option<String>,
    pub track_changes: bool,
    pub track_views: bool,
    pub allow_rename: bool,
    pub allow_copy: bool,
    pub allow_delete: bool,
    pub allow_print: bool,
    pub allow_email: bool,
    pub allow_attach: bool,
    pub allow_import: bool,
    pub allow_export: bool,
    pub allow_share: bool,
    pub allow_submit: bool,
    pub allow_cancel: bool,
    pub allow_amend: bool,
    pub is_submittable: bool,
    pub is_tree: bool,
    pub is_virtual: bool,
    pub is_single: bool,
    pub is_child_table: bool,
    pub indexes: Vec<IndexDef>,
    pub search_fields: Vec<String>,
    pub sort_field: Option<String>,
    pub sort_order: Option<String>,
    pub image_field: Option<String>,
    pub title_field: Option<String>,
    pub subject_field: Option<String>,
    pub sender_field: Option<String>,
    pub recipient_field: Option<String>,
    pub content_field: Option<String>,
    pub date_field: Option<String>,
    pub color_field: Option<String>,
    pub icon_field: Option<String>,
    pub custom: bool,
    pub beta: bool,
    pub hidden: bool,
    pub read_only: bool,
    pub quick_entry: bool,
    /// The `in_*` UI flags, parsed and validated by [`InFlags::set`].
    pub in_flags: InFlags,
}

impl Parse for DoctypeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = Self::default();

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            let _: Token![=] = input.parse()?;

            match ident.to_string().as_str() {
                "name" => args.name = Some(input.parse::<LitStr>()?.value()),
                "module" => args.module = Some(input.parse::<LitStr>()?.value()),
                "description" => args.description = Some(input.parse::<LitStr>()?.value()),
                "permissions" => {
                    let content;
                    syn::bracketed!(content in input);
                    args.permissions = content
                        .parse_terminated(PermissionRule::parse, Token![,])?
                        .into_iter()
                        .collect();
                }
                "naming" => args.naming = Some(input.parse::<LitStr>()?.value()),
                "naming_type" => parse_naming_type(input, &mut args.naming_type)?,
                "naming_field" => args.naming_field = Some(input.parse::<LitStr>()?.value()),
                "workflow" => args.workflow = Some(input.parse::<LitStr>()?.value()),
                "track_changes" => args.track_changes = input.parse::<syn::LitBool>()?.value(),
                "track_views" => args.track_views = input.parse::<syn::LitBool>()?.value(),
                "allow_rename" => args.allow_rename = input.parse::<syn::LitBool>()?.value(),
                "allow_copy" => args.allow_copy = input.parse::<syn::LitBool>()?.value(),
                "allow_delete" => args.allow_delete = input.parse::<syn::LitBool>()?.value(),
                "allow_print" => args.allow_print = input.parse::<syn::LitBool>()?.value(),
                "allow_email" => args.allow_email = input.parse::<syn::LitBool>()?.value(),
                "allow_attach" => args.allow_attach = input.parse::<syn::LitBool>()?.value(),
                "allow_import" => args.allow_import = input.parse::<syn::LitBool>()?.value(),
                "allow_export" => args.allow_export = input.parse::<syn::LitBool>()?.value(),
                "allow_share" => args.allow_share = input.parse::<syn::LitBool>()?.value(),
                "allow_submit" => args.allow_submit = input.parse::<syn::LitBool>()?.value(),
                "allow_cancel" => args.allow_cancel = input.parse::<syn::LitBool>()?.value(),
                "allow_amend" => args.allow_amend = input.parse::<syn::LitBool>()?.value(),
                "is_submittable" => args.is_submittable = input.parse::<syn::LitBool>()?.value(),
                "is_tree" => args.is_tree = input.parse::<syn::LitBool>()?.value(),
                "is_virtual" => args.is_virtual = input.parse::<syn::LitBool>()?.value(),
                "is_single" => args.is_single = input.parse::<syn::LitBool>()?.value(),
                "is_child_table" => args.is_child_table = input.parse::<syn::LitBool>()?.value(),
                "indexes" => {
                    let content;
                    syn::bracketed!(content in input);
                    args.indexes = content
                        .parse_terminated(IndexDef::parse, Token![,])?
                        .into_iter()
                        .collect();
                }
                "search_fields" => {
                    let content;
                    syn::bracketed!(content in input);
                    args.search_fields = content
                        .parse_terminated(syn::parse::ParseBuffer::parse::<LitStr>, Token![,])?
                        .into_iter()
                        .map(|s| s.value())
                        .collect();
                }
                "sort_field" => args.sort_field = Some(input.parse::<LitStr>()?.value()),
                "sort_order" => args.sort_order = Some(input.parse::<LitStr>()?.value()),
                "image_field" => args.image_field = Some(input.parse::<LitStr>()?.value()),
                "title_field" => args.title_field = Some(input.parse::<LitStr>()?.value()),
                "subject_field" => args.subject_field = Some(input.parse::<LitStr>()?.value()),
                "sender_field" => args.sender_field = Some(input.parse::<LitStr>()?.value()),
                "recipient_field" => args.recipient_field = Some(input.parse::<LitStr>()?.value()),
                "content_field" => args.content_field = Some(input.parse::<LitStr>()?.value()),
                "date_field" => args.date_field = Some(input.parse::<LitStr>()?.value()),
                "color_field" => args.color_field = Some(input.parse::<LitStr>()?.value()),
                "icon_field" => args.icon_field = Some(input.parse::<LitStr>()?.value()),
                "custom" => args.custom = input.parse::<syn::LitBool>()?.value(),
                "beta" => args.beta = input.parse::<syn::LitBool>()?.value(),
                "hidden" => args.hidden = input.parse::<syn::LitBool>()?.value(),
                "read_only" => args.read_only = input.parse::<syn::LitBool>()?.value(),
                "quick_entry" => args.quick_entry = input.parse::<syn::LitBool>()?.value(),
                _ => {
                    let flag_name = ident.to_string();
                    if !flag_name.starts_with("in_") {
                        return Err(syn::Error::new(
                            ident.span(),
                            format!("Unknown attribute: {flag_name}"),
                        ));
                    }
                    let enabled = input.parse::<syn::LitBool>()?.value();
                    if !args.in_flags.set(&flag_name, enabled) {
                        return Err(syn::Error::new(
                            ident.span(),
                            format!("Unknown attribute: {flag_name}"),
                        ));
                    }
                }
            }

            if !input.is_empty() {
                let _: Token![,] = input.parse()?;
            }
        }

        Ok(args)
    }
}

/// `naming_type` accepts both spellings the attribute grammar has used:
/// the bare word `series` (what the docs write) and the quoted `"series"`
/// (what a `LitStr` parser used to demand). Both are read here, so a
/// document written either way compiles; anything else is refused with the
/// attribute's own message.
fn parse_naming_type(input: ParseStream, slot: &mut Option<String>) -> syn::Result<()> {
    if input.peek(LitStr) {
        let value: LitStr = input.parse()?;
        *slot = Some(value.value());
        return Ok(());
    }
    let value: Ident = input.parse().map_err(|_| {
        syn::Error::new(
            input.span(),
            "naming_type expects a word (`series`, `naming_series`, `auto`, `field`, \
             `prompt`, `expression`, `expression_old_style`, `random`, `script`) or a quoted string",
        )
    })?;
    *slot = Some(value.to_string());
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct IndexDef {
    pub name: String,
    pub fields: Vec<String>,
    pub unique: bool,
    pub index_type: IndexType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexType {
    BTree,
    Hash,
    Gist,
    Gin,
    SpGist,
    Brin,
}

impl Parse for IndexDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut index = Self {
            name: String::new(),
            fields: Vec::new(),
            unique: false,
            index_type: IndexType::BTree,
        };

        let content;
        syn::braced!(content in input);

        while !content.is_empty() {
            let ident: Ident = content.parse()?;
            let _: Token![=] = content.parse()?;

            match ident.to_string().as_str() {
                "name" => index.name = content.parse::<LitStr>()?.value(),
                "fields" => {
                    let arr_content;
                    syn::bracketed!(arr_content in content);
                    index.fields = arr_content
                        .parse_terminated(syn::parse::ParseBuffer::parse::<LitStr>, Token![,])?
                        .into_iter()
                        .map(|s| s.value())
                        .collect();
                }
                "unique" => index.unique = content.parse::<syn::LitBool>()?.value(),
                "type" => {
                    let t: LitStr = content.parse()?;
                    index.index_type = match t.value().as_str() {
                        "btree" => IndexType::BTree,
                        "hash" => IndexType::Hash,
                        "gist" => IndexType::Gist,
                        "gin" => IndexType::Gin,
                        "spgist" => IndexType::SpGist,
                        "brin" => IndexType::Brin,
                        _ => return Err(syn::Error::new(t.span(), "Invalid index type")),
                    };
                }
                _ => return Err(syn::Error::new(ident.span(), "Unknown index field")),
            }

            if !content.is_empty() {
                let _: Token![,] = content.parse()?;
            }
        }

        Ok(index)
    }
}
