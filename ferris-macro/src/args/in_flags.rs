//! The DocType-level `in_*` UI flags.
//!
//! The list is the **single source of truth**: it generates the
//! [`InFlags`] set, the name → flag parser, and the `DoctypeMeta` initialiser
//! fragment, so a flag cannot exist in the parser but not in the emitted
//! metadata (or vice versa). The ~46 `in_*` fields that were threaded through
//! struct → parser → `DoctypeMeta` but never read anywhere in the workspace are
//! gone; only the flags the framework actually consumes are listed.
//!
//! The macro lives in its own file because it is the one piece of the argument
//! grammar shared by more than one type: everything else is a plain `Parse`
//! impl over a single struct.

use std::collections::BTreeSet;

use syn::Ident;

/// Declare the `in_*` flags the `#[doctype]` attribute accepts.
macro_rules! doctype_in_flags {
    ($($flag:ident),+ $(,)?) => {
        /// The `in_*` flags enabled on a `#[doctype]`.
        ///
        /// A set rather than a struct of bools: the flags are an unordered
        /// tuple of independent switches, so fixed fields buy nothing and would
        /// additionally trip `clippy::struct_excessive_bools`.
        #[derive(Debug, Default)]
        pub(crate) struct InFlags(BTreeSet<&'static str>);

        impl InFlags {
            /// Record one `in_*` attribute. Returns `false` for a name the
            /// macro does not know, so the caller can emit a hard error instead
            /// of silently dropping the attribute.
            pub(crate) fn set(&mut self, name: &str, enabled: bool) -> bool {
                $(
                    if name == stringify!($flag) {
                        if enabled {
                            self.0.insert(stringify!($flag));
                        } else {
                            self.0.remove(stringify!($flag));
                        }
                        return true;
                    }
                )+
                false
            }

            /// Emit the `in_*: true` fields for a `DoctypeMeta` literal. Only
            /// enabled flags are emitted; `DoctypeMeta`'s `Default` supplies
            /// `false` for the rest.
            pub(crate) fn meta_fields(&self) -> proc_macro2::TokenStream {
                // `iter().map(..).collect()` is the clearest form for a
                // `TokenStream`; the closure-to-method-call lint does not help.
                #[allow(clippy::redundant_closure_for_method_calls)]
                let tokens: proc_macro2::TokenStream = self
                    .0
                    .iter()
                    .map(|name| {
                        let ident = Ident::new(name, proc_macro2::Span::call_site());
                        quote::quote!(#ident: true,)
                    })
                    .collect();
                tokens
            }
        }
    };
}

doctype_in_flags! {
    in_create,
    in_list_view,
    in_dialog,
    in_global_search,
    in_calendar,
    in_kanban,
    in_gantt,
    in_tree_view,
    in_report_view,
}
