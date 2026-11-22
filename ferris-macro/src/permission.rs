//! Permission macro expansion.
//!
//! `#[permission(...)]` used to be a pass-through stub: the attribute parsed
//! and then **discarded** its arguments, so `#[permission(actions=["Read"],
//! roles=["Sales"])]` compiled into a plain struct and the rule was silently
//! never installed. This expansion emits the annotated item plus a
//! `register_<snake_case>_permission()` free function (and a
//! `register_permission()` associated function) that installs the declared
//! grants into the process-wide permission engine as a `PolicyStatement`.
//!
//! The statement uses the same two resource shapes as `doc_type_policy`:
//! `(doctype, "*")` for whole-doc queries and `("doctype", doctype)` for
//! doctype-level probes — mirroring what the engine actually checks
//! (`ferris-core/src/permissions.rs`).

use crate::PermissionArgs;
use crate::util::{parse_condition, to_snake_case};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, Ident, ItemStruct};

/// The actions `ferris_core::permissions::Permission` can express. Kept in sync
/// with the enum; validated here so a typo'd action is a compile-time error
/// instead of a rule that matches nothing at runtime.
const PERMISSION_ACTIONS: &[&str] = &[
    "Read", "Write", "Create", "Delete", "Submit", "Cancel", "Amend", "Print", "Email", "Share",
    "Report", "Import", "Export", "Manage",
];

pub fn expand(args: &PermissionArgs, input: &ItemStruct) -> Result<TokenStream, Error> {
    let struct_name = &input.ident;
    let resource = args
        .resource
        .clone()
        .unwrap_or_else(|| struct_name.to_string());

    if args.actions.is_empty() {
        return Err(Error::new_spanned(
            struct_name,
            "permission: `actions` must list at least one action (e.g. [\"Read\"])",
        ));
    }
    if args.roles.is_empty() {
        return Err(Error::new_spanned(
            struct_name,
            "permission: `roles` must list at least one role (e.g. [\"Sales Manager\"])",
        ));
    }

    // Validate every action at expansion time and map it to the Permission
    // variant. Unrecognised names are macro errors, never silent no-ops.
    let actions: Vec<TokenStream> = args
        .actions
        .iter()
        .map(|action| {
            if !PERMISSION_ACTIONS.contains(&action.as_str()) {
                return Err(Error::new_spanned(
                    struct_name,
                    format!(
                        "permission: unknown action `{action}`; expected one of {}",
                        PERMISSION_ACTIONS.join(", ")
                    ),
                ));
            }
            let variant = Ident::new(action, struct_name.span());
            Ok(quote!(ferris_core::permissions::Permission::#variant))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    // `"*"`/`"All"` is the catch-all role; everything else is a plain role.
    // Conditions are still parsed through the same safe-equality grammar the
    // `#[workflow]` macro uses — never evaluated user code.
    let principals: Vec<TokenStream> = args
        .roles
        .iter()
        .map(|role| {
            if role == "*" || role == "All" {
                quote!(ferris_core::permissions::Principal::everyone())
            } else {
                quote!(ferris_core::permissions::Principal::role(#role))
            }
        })
        .collect();

    let condition = match &args.condition {
        Some(expr) => {
            let (field, value_token) = parse_condition(expr, "permission", struct_name.span())?;
            Some(quote! { .with_condition(#field.to_string(), #value_token) })
        }
        None => None,
    };

    let register_fn = syn::Ident::new(
        &format!(
            "register_{}_permission",
            to_snake_case(&struct_name.to_string())
        ),
        struct_name.span(),
    );

    let statement = quote! {
        ferris_core::permissions::PolicyStatement::allow(
            vec![#(#principals),*],
            vec![#(#actions),*],
            vec![
                ferris_core::permissions::Resource::new(#resource, "*"),
                ferris_core::permissions::Resource::new("doctype", #resource),
            ],
        )#condition
    };

    Ok(quote! {
        #input

        /// Install this permission rule into the process-wide engine.
        ///
        /// Idempotent: the first call installs the policy, later calls are
        /// no-ops. Re-running registration (module init plus a test, say) used
        /// to append a duplicate policy per call, which inflated the engine's
        /// policy list with rules that were already there.
        pub fn #register_fn() -> ferris_core::Result<()> {
            static REGISTERED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
            REGISTERED.get_or_init(|| {
                let policy = ferris_core::permissions::Policy::new(vec![#statement]);
                // The guard only needs to live for `add_policy`; scoping it into
                // the closure releases the engine lock as it returns.
                let mut engine = ferris_core::permissions::global_engine_ref()
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                engine.add_policy(policy);
            });
            Ok(())
        }

        impl #struct_name {
            /// Register this permission rule in the process-wide engine.
            pub fn register_permission() -> ferris_core::Result<()> {
                #register_fn()
            }
        }
    })
}
