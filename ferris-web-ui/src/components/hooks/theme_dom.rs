//! The one place the Desk writes the active theme to the DOM and storage.
//!
//! `ferris_web::theme` owns the state (`ThemeCtx`) and `ferris-web-ui` owns
//! the components that switch it, but the paint itself must have a single
//! implementation: two copies drift, and the copy that forgets `color-scheme`
//! leaves native scrollbars and date pickers in the other mode. This module is
//! that implementation; `ferris_web::theme` calls it instead of keeping its own
//! copy of the `<style>` injection.
//!
//! The storage key is [`ferris_web_shared::theme::THEME_STORAGE_KEY`] — the same
//! key [`super::use_theme_mode`] writes, so a toggle survives a reload.

pub use ferris_web_shared::theme::{ThemeSpec, is_dark_theme, resolve_theme, theme_css};

/// Id of the injected `<style>` that applies the active palette.
pub const THEME_STYLE_ID: &str = "ferris-theme";

/// Read the persisted theme id.
#[must_use]
// The native stub is trivially `const`; the wasm body is not, and a
// cfg-dependent `const fn` signature would not compile for the browser.
#[cfg_attr(not(target_arch = "wasm32"), allow(clippy::missing_const_for_fn))]
pub fn stored_theme_id() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()?
            .local_storage()
            .ok()??
            .get_item(ferris_web_shared::theme::THEME_STORAGE_KEY)
            .ok()?
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

/// Persist the active theme id.
// Same cfg split as [`stored_theme_id`]: const offline, DOM-bound in the browser.
#[cfg_attr(not(target_arch = "wasm32"), allow(clippy::missing_const_for_fn))]
pub fn persist_theme_id(id: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(storage) = web_sys::window()
        .and_then(|window| window.local_storage().ok())
        .flatten()
    {
        let _ = storage.set_item(ferris_web_shared::theme::THEME_STORAGE_KEY, id);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
}

/// Paint `spec` on `<html>`: `data-theme`, `color-scheme` and the injected
/// palette `<style>`.
///
/// `ThemeSpec::dark` decides `color-scheme` so a registered dark theme
/// (`noche`) gets native dark widgets too, instead of only the tokens.
// Same cfg split as [`stored_theme_id`]: the native build is a no-op.
#[cfg_attr(not(target_arch = "wasm32"), allow(clippy::missing_const_for_fn))]
pub fn apply_theme(spec: &ThemeSpec) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast as _;

        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let Some(root) = document.document_element() else {
            return;
        };
        let _ = root.set_attribute("data-theme", &spec.id);

        if let Some(html) = root.dyn_into::<web_sys::HtmlElement>().ok() {
            let _ = html
                .style()
                .set_property("color-scheme", if spec.dark { "dark" } else { "light" });
        }

        let style = match document.get_element_by_id(THEME_STYLE_ID) {
            Some(existing) => existing,
            None => {
                let Some(element) = document.create_element("style").ok() else {
                    return;
                };
                element.set_id(THEME_STYLE_ID);
                if let Some(head) = document.head() {
                    let _ = head.append_child(&element);
                }
                element
            }
        };
        style.set_text_content(Some(&theme_css(spec)));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = spec;
}

#[cfg(test)]
mod tests {
    use super::{resolve_theme, theme_css};

    #[test]
    fn apply_path_survives_a_missing_dom_off_browser() {
        // The native build must be a no-op, not a panic: the SSR pass renders
        // the same components.
        super::apply_theme(&ferris_web_shared::theme::builtin_themes()[0]);
        assert!(super::stored_theme_id().is_none());
        super::persist_theme_id("dark");
    }

    #[test]
    fn palette_css_is_the_shared_one() {
        let themes = ferris_web_shared::theme::builtin_themes();
        let Some(dark) = resolve_theme(&themes, Some("dark")) else {
            return;
        };
        let css = theme_css(&dark);
        assert!(css.contains("--background:"));
        assert!(dark.dark);
    }
}
