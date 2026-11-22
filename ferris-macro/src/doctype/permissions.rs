//! `permissions = [ … ]` → `ferris_core::PermissionRule` literals.
//!
//! Split out of the expansion module so the metadata/impl generation and the
//! permission shape are reviewed separately.

use crate::args::PermissionRule;
use proc_macro2::TokenStream;
use quote::quote;

pub(super) fn generate_permission_consts(rules: &[PermissionRule]) -> Vec<TokenStream> {
    rules
        .iter()
        .map(|r| {
            let role = &r.role;
            let read = r.read;
            let write = r.write;
            let create = r.create;
            let delete = r.delete;
            let submit = r.submit;
            let cancel = r.cancel;
            let amend = r.amend;
            let print = r.print;
            let email = r.email;
            let share = r.share;
            let report = r.report;
            let import = r.import;
            let export = r.export;
            let permlevel = r.permlevel;
            let if_owner = r.if_owner;
            let apply_to_all = r.apply_to_all_doctypes;

            quote! {
                ferris_core::PermissionRule {
                    role: #role.to_string(),
                    read: #read,
                    write: #write,
                    create: #create,
                    delete: #delete,
                    submit: #submit,
                    cancel: #cancel,
                    amend: #amend,
                    print: #print,
                    email: #email,
                    share: #share,
                    report: #report,
                    import: #import,
                    export: #export,
                    permlevel: #permlevel,
                    if_owner: #if_owner,
                    apply_to_all_doctypes: #apply_to_all,
                    custom_permissions: Vec::new(),
                }
            }
        })
        .collect()
}
