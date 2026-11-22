//! `#[job(...)]` arguments.

use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token};

#[derive(Debug, Default)]
pub(crate) struct JobArgs {
    pub name: Option<String>,
    pub queue: Option<String>,
    pub priority: Option<i32>,
    pub max_retries: Option<u32>,
    pub timeout_secs: Option<u64>,
}

impl Parse for JobArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = Self::default();

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            let _: Token![=] = input.parse()?;

            match ident.to_string().as_str() {
                "name" => args.name = Some(input.parse::<LitStr>()?.value()),
                "queue" => args.queue = Some(input.parse::<LitStr>()?.value()),
                "priority" => args.priority = Some(input.parse::<syn::LitInt>()?.base10_parse()?),
                "max_retries" => {
                    args.max_retries = Some(input.parse::<syn::LitInt>()?.base10_parse()?);
                }
                "timeout_secs" => {
                    args.timeout_secs = Some(input.parse::<syn::LitInt>()?.base10_parse()?);
                }
                _ => return Err(syn::Error::new(ident.span(), "Unknown job attribute")),
            }

            if !input.is_empty() {
                let _: Token![,] = input.parse()?;
            }
        }

        Ok(args)
    }
}
