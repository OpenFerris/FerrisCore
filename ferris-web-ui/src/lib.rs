//! Leptos component library for the Ferris Desk.
//!
//! This crate is the UI leaf of the `ferris-web` graph: components, hooks,
//! style helpers and the icon wrapper, and nothing else. It carries no routing,
//! no data fetching and no engine bridge, so editing a component cannot force a
//! rebuild of the layers above it.
//!
//! The module tree mirrors `ferris-web::components` on purpose, so that crate
//! re-exports these modules under their historical paths and callers keep
//! compiling unchanged.

#![forbid(unsafe_code)]

pub mod components {
    pub mod constants;
    pub mod desk;
    pub mod dropdown_builder;
    pub mod hooks;
    pub mod ui;
    pub mod utils;
}

pub use components::{constants, desk, dropdown_builder, hooks, ui, utils};

pub mod catalog;
pub mod i18n;

/// The per-response CSP nonce.
///
/// Server-side this is the nonce the middleware minted for the current response
/// (Leptos provides it via context). On the client there is no context: the
/// value only exists on the document the server rendered, stamped onto the
/// scripts it emitted. Components that build an inline `<script>` *after*
/// hydration (dialogs, popovers, selects, menubars …) must reuse that same nonce
/// or the strict `script-src 'nonce-…'` policy drops them and the widget
/// silently never initialises.
///
/// The client value is read once per document and cached: it cannot change for
/// the life of the document, and every inline script the desk emits would
/// otherwise re-walk the DOM.
pub fn csp_nonce() -> Option<String> {
    #[cfg(feature = "ssr")]
    {
        leptos::nonce::use_nonce().map(|nonce| nonce.to_string())
    }
    #[cfg(not(feature = "ssr"))]
    {
        use std::cell::OnceCell;

        thread_local! {
            static NONCE: OnceCell<Option<String>> = const { OnceCell::new() };
        }

        fn read_nonce() -> Option<String> {
            use wasm_bindgen::JsCast as _;

            let doc = web_sys::window()?.document()?;
            let el = doc.query_selector("script[nonce]").ok().flatten()?;
            // Read the IDL property, not the attribute: after parsing the browser
            // blanks the `nonce` attribute and keeps the real value in the
            // element's internal slot.
            let script = el.dyn_into::<web_sys::HtmlScriptElement>().ok()?;
            let nonce = script.nonce();
            (!nonce.is_empty()).then_some(nonce)
        }

        NONCE.with(|cell| cell.get_or_init(read_nonce).clone())
    }
}
