//! `#[permission(...)]` arguments.
//!
//! [`PermissionRule`] is also the element type of `#[doctype(permissions = …)]`,
//! so the two macros share one rule shape instead of two drifting copies.

use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token};

/// One `permissions = [ … ]` entry.
///
/// Like [`DoctypeArgs`], this is a bag of independent grant flags, so the
/// `struct_excessive_bools` heuristic does not apply.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub(crate) struct PermissionRule {
    pub role: String,
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub delete: bool,
    pub submit: bool,
    pub cancel: bool,
    pub amend: bool,
    pub print: bool,
    pub email: bool,
    pub share: bool,
    pub report: bool,
    pub import: bool,
    pub export: bool,
    pub permlevel: u8,
    pub if_owner: bool,
    pub apply_to_all_doctypes: bool,
}

impl Parse for PermissionRule {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut rule = Self {
            role: String::new(),
            read: false,
            write: false,
            create: false,
            delete: false,
            submit: false,
            cancel: false,
            amend: false,
            print: false,
            email: false,
            share: false,
            report: false,
            import: false,
            export: false,
            permlevel: 0,
            if_owner: false,
            apply_to_all_doctypes: false,
        };

        let content;
        syn::braced!(content in input);

        while !content.is_empty() {
            let ident: Ident = content.parse()?;
            let _: Token![=] = content.parse()?;

            match ident.to_string().as_str() {
                "role" => rule.role = content.parse::<LitStr>()?.value(),
                "read" => rule.read = content.parse::<syn::LitBool>()?.value(),
                "write" => rule.write = content.parse::<syn::LitBool>()?.value(),
                "create" => rule.create = content.parse::<syn::LitBool>()?.value(),
                "delete" => rule.delete = content.parse::<syn::LitBool>()?.value(),
                "submit" => rule.submit = content.parse::<syn::LitBool>()?.value(),
                "cancel" => rule.cancel = content.parse::<syn::LitBool>()?.value(),
                "amend" => rule.amend = content.parse::<syn::LitBool>()?.value(),
                "print" => rule.print = content.parse::<syn::LitBool>()?.value(),
                "email" => rule.email = content.parse::<syn::LitBool>()?.value(),
                "share" => rule.share = content.parse::<syn::LitBool>()?.value(),
                "report" => rule.report = content.parse::<syn::LitBool>()?.value(),
                "import" => rule.import = content.parse::<syn::LitBool>()?.value(),
                "export" => rule.export = content.parse::<syn::LitBool>()?.value(),
                "permlevel" => rule.permlevel = content.parse::<syn::LitInt>()?.base10_parse()?,
                "if_owner" => rule.if_owner = content.parse::<syn::LitBool>()?.value(),
                "apply_to_all_doctypes" => {
                    rule.apply_to_all_doctypes = content.parse::<syn::LitBool>()?.value();
                }
                _ => return Err(syn::Error::new(ident.span(), "Unknown permission field")),
            }

            if !content.is_empty() {
                let _: Token![,] = content.parse()?;
            }
        }

        Ok(rule)
    }
}

#[derive(Debug, Default)]
pub(crate) struct PermissionArgs {
    pub resource: Option<String>,
    pub actions: Vec<String>,
    pub roles: Vec<String>,
    pub condition: Option<String>,
}

impl Parse for PermissionArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = Self::default();

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            let _: Token![=] = input.parse()?;

            match ident.to_string().as_str() {
                "resource" => args.resource = Some(input.parse::<LitStr>()?.value()),
                "actions" => {
                    let content;
                    syn::bracketed!(content in input);
                    args.actions = content
                        .parse_terminated(syn::parse::ParseBuffer::parse::<LitStr>, Token![,])?
                        .into_iter()
                        .map(|s| s.value())
                        .collect();
                }
                "roles" => {
                    let content;
                    syn::bracketed!(content in input);
                    args.roles = content
                        .parse_terminated(syn::parse::ParseBuffer::parse::<LitStr>, Token![,])?
                        .into_iter()
                        .map(|s| s.value())
                        .collect();
                }
                "condition" => args.condition = Some(input.parse::<LitStr>()?.value()),
                _ => {
                    return Err(syn::Error::new(
                        ident.span(),
                        "Unknown permission attribute",
                    ));
                }
            }

            if !input.is_empty() {
                let _: Token![,] = input.parse()?;
            }
        }

        Ok(args)
    }
}
