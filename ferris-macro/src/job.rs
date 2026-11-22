//! Job macro expansion.
//!
//! `#[job(...)]` used to be a pass-through stub: the attribute parsed and then
//! **discarded** its arguments, so `#[job(name="send_invoice", queue="billing")]`
//! compiled into a plain struct and no job was ever representable with those
//! settings. This expansion emits the annotated item plus a
//! `job(payload)` associated function that builds a `ferris_jobs::Job` carrying
//! the declared name, queue, priority, retry budget and timeout — the exact
//! knobs the queue worker reads off `Job`.

use crate::args::JobArgs;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, ItemStruct};

pub fn expand(args: &JobArgs, input: &ItemStruct) -> TokenStream {
    let struct_name = &input.ident;
    let name = args.name.clone().unwrap_or_else(|| struct_name.to_string());
    let queue = args.queue.clone().unwrap_or_else(|| "default".to_string());
    let max_retries = args.max_retries.unwrap_or(3);
    let timeout_secs = args.timeout_secs.unwrap_or(300);

    // `priority` is declared as an integer; `JobPriority` is a fixed ladder, so
    // the tiers map onto it the way `Job::with_priority` consumers expect.
    let priority_variant = match args.priority {
        Some(p) if p >= 20 => "Critical",
        Some(p) if p >= 10 => "High",
        Some(p) if p <= -10 => "Low",
        _ => "Normal",
    };
    let priority = Ident::new(priority_variant, struct_name.span());

    quote! {
        #input

        impl #struct_name {
            /// Build a `ferris_jobs::Job` carrying this job's configured name,
            /// queue, priority, retry budget and timeout.
            pub fn job(payload: ferris_core::Value) -> ferris_jobs::Job {
                ferris_jobs::Job::new(#name.to_string(), payload)
                    .with_queue(#queue.to_string())
                    .with_priority(ferris_jobs::JobPriority::#priority)
                    .with_max_retries(#max_retries)
                    .with_timeout(std::time::Duration::from_secs(#timeout_secs))
            }
        }
    }
}
