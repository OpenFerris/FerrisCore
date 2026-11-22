//! `#[workflow(...)]` arguments.

use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token};

#[derive(Debug, Default)]
pub(crate) struct WorkflowArgs {
    pub name: Option<String>,
    pub doctype: Option<String>,
    pub initial_state: Option<String>,
    pub states: Vec<WorkflowState>,
    pub transitions: Vec<WorkflowTransition>,
}

impl Parse for WorkflowArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = Self::default();

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            let _: Token![=] = input.parse()?;

            match ident.to_string().as_str() {
                "name" => args.name = Some(input.parse::<LitStr>()?.value()),
                "doctype" => args.doctype = Some(input.parse::<LitStr>()?.value()),
                "initial_state" => args.initial_state = Some(input.parse::<LitStr>()?.value()),
                "states" => {
                    let content;
                    syn::bracketed!(content in input);
                    args.states = content
                        .parse_terminated(WorkflowState::parse, Token![,])?
                        .into_iter()
                        .collect();
                }
                "transitions" => {
                    let content;
                    syn::bracketed!(content in input);
                    args.transitions = content
                        .parse_terminated(WorkflowTransition::parse, Token![,])?
                        .into_iter()
                        .collect();
                }
                _ => return Err(syn::Error::new(ident.span(), "Unknown workflow attribute")),
            }

            if !input.is_empty() {
                let _: Token![,] = input.parse()?;
            }
        }

        Ok(args)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkflowState {
    pub name: String,
    pub label: String,
    pub style: String,
    pub doc_status: String,
    pub allow_edit: Vec<String>,
    pub allowed_actions: Vec<String>,
    pub auto_assign: Option<String>,
}

impl Parse for WorkflowState {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut state = Self {
            name: String::new(),
            label: String::new(),
            style: "default".into(),
            doc_status: "draft".into(),
            allow_edit: Vec::new(),
            allowed_actions: Vec::new(),
            auto_assign: None,
        };

        let content;
        syn::braced!(content in input);

        while !content.is_empty() {
            let ident: Ident = content.parse()?;
            let _: Token![=] = content.parse()?;

            match ident.to_string().as_str() {
                "name" => state.name = content.parse::<LitStr>()?.value(),
                "label" => state.label = content.parse::<LitStr>()?.value(),
                "style" => state.style = content.parse::<LitStr>()?.value(),
                "doc_status" => state.doc_status = content.parse::<LitStr>()?.value(),
                "allow_edit" => {
                    let arr_content;
                    syn::bracketed!(arr_content in content);
                    state.allow_edit = arr_content
                        .parse_terminated(syn::parse::ParseBuffer::parse::<LitStr>, Token![,])?
                        .into_iter()
                        .map(|s| s.value())
                        .collect();
                }
                "allowed_actions" => {
                    let arr_content;
                    syn::bracketed!(arr_content in content);
                    state.allowed_actions = arr_content
                        .parse_terminated(syn::parse::ParseBuffer::parse::<LitStr>, Token![,])?
                        .into_iter()
                        .map(|s| s.value())
                        .collect();
                }
                "auto_assign" => state.auto_assign = Some(content.parse::<LitStr>()?.value()),
                _ => {
                    return Err(syn::Error::new(
                        ident.span(),
                        "Unknown workflow state field",
                    ));
                }
            }

            if !content.is_empty() {
                let _: Token![,] = content.parse()?;
            }
        }

        Ok(state)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkflowTransition {
    pub name: String,
    pub from_state: String,
    pub to_state: String,
    pub action: String,
    pub condition: Option<String>,
    pub allowed_roles: Vec<String>,
    pub required_fields: Vec<String>,
    pub notify: bool,
}

impl Parse for WorkflowTransition {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut transition = Self {
            name: String::new(),
            from_state: String::new(),
            to_state: String::new(),
            action: String::new(),
            condition: None,
            allowed_roles: Vec::new(),
            required_fields: Vec::new(),
            notify: false,
        };

        let content;
        syn::braced!(content in input);

        while !content.is_empty() {
            let ident: Ident = content.parse()?;
            let _: Token![=] = content.parse()?;

            match ident.to_string().as_str() {
                "name" => transition.name = content.parse::<LitStr>()?.value(),
                "from" => transition.from_state = content.parse::<LitStr>()?.value(),
                "to" => transition.to_state = content.parse::<LitStr>()?.value(),
                "action" => transition.action = content.parse::<LitStr>()?.value(),
                "condition" => transition.condition = Some(content.parse::<LitStr>()?.value()),
                "allowed_roles" => {
                    let arr_content;
                    syn::bracketed!(arr_content in content);
                    transition.allowed_roles = arr_content
                        .parse_terminated(syn::parse::ParseBuffer::parse::<LitStr>, Token![,])?
                        .into_iter()
                        .map(|s| s.value())
                        .collect();
                }
                "required_fields" => {
                    let arr_content;
                    syn::bracketed!(arr_content in content);
                    transition.required_fields = arr_content
                        .parse_terminated(syn::parse::ParseBuffer::parse::<LitStr>, Token![,])?
                        .into_iter()
                        .map(|s| s.value())
                        .collect();
                }
                "notify" => transition.notify = content.parse::<syn::LitBool>()?.value(),
                _ => {
                    return Err(syn::Error::new(
                        ident.span(),
                        "Unknown workflow transition field",
                    ));
                }
            }

            if !content.is_empty() {
                let _: Token![,] = content.parse()?;
            }
        }

        Ok(transition)
    }
}
