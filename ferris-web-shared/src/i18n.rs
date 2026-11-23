//! Locale context for the Desk, shared by `ferris-web` and `ferris-web-ui`.
//!
//! The type is plain data on purpose: this crate has no Leptos and no
//! `web-sys`, so both the SSR server (through `ferris-web`) and the UI
//! primitives (through `ferris-web-ui`) can name the same context type without
//! inverting the dependency edge. The Leptos helpers that read the context live
//! in each crate (`ferris_web::i18n::t`, `ferris_web_ui::i18n::t`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The locale and the flattened messages the Desk renders with.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeskI18n {
    /// Canonical tag, e.g. `es` or `es-ES`.
    pub locale: String,
    /// Effective messages: English base plus the locale's overrides.
    pub messages: BTreeMap<String, String>,
    /// Locales the deployment advertises; the language picker renders these.
    #[serde(default)]
    pub supported: Vec<String>,
}

/// The two operations every catalogue carrier exposes.
///
/// `DeskI18n` (the owned data) and the `Copy` handles in `ferris-web`/
/// `ferris-web-ui` all implement it, so a helper can take `&impl Translate`
/// and be called from a component (handle) or a test (plain catalogue).
pub trait Translate {
    fn t(&self, key: &str) -> String;
    fn t_with(&self, key: &str, args: &[(&str, &str)]) -> String;
}

impl Translate for DeskI18n {
    fn t(&self, key: &str) -> String {
        DeskI18n::t(self, key)
    }

    fn t_with(&self, key: &str, args: &[(&str, &str)]) -> String {
        DeskI18n::t_with(self, key, args)
    }
}

impl DeskI18n {
    /// Build the context from the server's JSON catalogue.
    #[must_use]
    pub fn from_catalog(locale: &str, catalog: &serde_json::Value) -> Self {
        let messages = catalog
            .as_object()
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|text| (key.clone(), text.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            locale: locale.to_string(),
            messages,
            supported: Vec::new(),
        }
    }

    /// Attach the deployment's advertised locales.
    #[must_use]
    pub fn with_supported(mut self, supported: Vec<String>) -> Self {
        self.supported = supported;
        self
    }

    /// The message for `key`, or the key itself when the catalogue lacks it.
    #[must_use]
    pub fn t(&self, key: &str) -> String {
        self.messages
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    /// Whether the catalogue carries `key`.
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.messages.contains_key(key)
    }

    /// Translate `key` and replace `{name}` placeholders from `args`.
    ///
    /// The server has the same operation on `ferris_core::I18n`; the Desk needs
    /// its own copy because the browser only carries the flattened catalogue.
    /// Unknown placeholders are left literal, which makes a typo visible.
    #[must_use]
    pub fn t_with(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.t(key);
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::DeskI18n;

    #[test]
    fn the_catalog_flattens_to_the_effective_messages() {
        let catalog = serde_json::json!({
            "login.title": "Ferris Desk",
            "login.submit": "Entrar",
            "not-a-string": 3
        });
        let i18n = DeskI18n::from_catalog("es", &catalog);
        assert_eq!(i18n.locale, "es");
        assert_eq!(i18n.t("login.title"), "Ferris Desk");
        assert_eq!(i18n.t("login.submit"), "Entrar");
        assert_eq!(i18n.t("missing.key"), "missing.key");
        assert!(i18n.has("login.submit"));
        assert!(!i18n.has("not-a-string"));
    }

    #[test]
    fn an_empty_catalog_still_returns_the_key() {
        let i18n = DeskI18n::default();
        assert_eq!(i18n.t("login.title"), "login.title");
    }
}
