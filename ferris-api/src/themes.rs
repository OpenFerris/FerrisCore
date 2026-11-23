//! Desk themes: API surface for the extensible theme registry.
//!
//! Two feeds are composed here:
//!
//! * the built-in `light`/`dark` themes, whose palette is the single Rust
//!   definition in [`ferris_schema::contract::theme::builtin_themes`] (shared
//!   with the wasm `hydrate` client and checked against `tailwind.css` by the
//!   `design_system` test);
//! * the extra themes an app registered through
//!   [`ferris_core::themes::register_theme`] / `register_themes` (the plugin host
//!   registers a `describe()`'s `"themes"` there, and this crate sits above both
//!   `ferris-core` and `ferris-schema`, so it is the one place allowed to combine
//!   them).
//!
//! The registry lives in `ferris-core` because `ferris-schema` depends on it and
//! the plugin host may not pull `ferris-schema` in; the built-in palette lives in
//! `ferris-schema` because it must reach the wasm build, which `ferris-core` may
//! not enter. Composition happens at this seam.

pub use ferris_core::themes::{Registration, register_theme, register_themes, registered_themes};
// The Desk deserializes `/api/themes` into `ferris_schema::contract::ThemeSpec`,
// so that is the type this module serves. The registry's own `ThemeSpec` (a
// different, structurally identical type, because `ferris-core` cannot depend on
// `ferris-schema`) is converted below and checked by the round-trip test.
pub use ferris_schema::contract::theme::{ThemeSpec, builtin_themes, dark_theme, light_theme};

use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use ferris_core::i18n::{I18n, Locale, MetaTranslate};

use crate::ApiState;
use crate::router::RequestLocale;

/// `GET /api/themes` — built-in themes first, then the app-registered ones,
/// labelled in the caller's language.
pub async fn themes(
    State(state): State<Arc<ApiState>>,
    RequestLocale(locale): RequestLocale,
) -> Json<Vec<ThemeSpec>> {
    Json(localize_themes(all_themes(), &state.i18n, &locale))
}

/// Translate each theme's label through `theme.<id>.label`.
#[must_use]
pub fn localize_themes(mut themes: Vec<ThemeSpec>, i18n: &I18n, locale: &Locale) -> Vec<ThemeSpec> {
    let t = MetaTranslate::new(i18n, locale);
    for theme in &mut themes {
        theme.label = t.theme_label(&theme.id, &theme.label).into_owned();
    }
    themes
}

/// Built-ins followed by registered extras; a registered theme whose id matches
/// a built-in replaces it (an app may restyle `light`/`dark`), otherwise it is
/// appended.
#[must_use]
pub fn all_themes() -> Vec<ThemeSpec> {
    let mut out = builtin_themes();
    for extra in registered_themes() {
        let spec = into_contract(extra);
        if let Some(existing) = out.iter_mut().find(|t| t.id == spec.id) {
            *existing = spec;
        } else {
            out.push(spec);
        }
    }
    out
}

/// Re-type a registry theme into the wire contract. The two structs carry the
/// same four fields (that is the whole point of the round-trip test below);
/// `ferris-core` cannot depend on `ferris-schema`, so the conversion lives here.
fn into_contract(theme: ferris_core::themes::ThemeSpec) -> ThemeSpec {
    ThemeSpec {
        id: theme.id,
        label: theme.label,
        dark: theme.dark,
        vars: theme.vars,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_come_first_and_registration_appends() {
        let _ = register_theme(ferris_core::themes::ThemeSpec {
            id: "api-test".into(),
            label: "API Test".into(),
            dark: false,
            vars: vec![("--background".into(), "oklch(0.5 0 0)".into())],
        });
        let all = all_themes();
        assert_eq!(all[0].id, "light");
        assert!(all.iter().any(|t| t.id == "api-test"));
    }

    #[test]
    fn registered_theme_overrides_builtin() {
        let _ = register_theme(ferris_core::themes::ThemeSpec {
            id: "dark".into(),
            label: "Dark v2".into(),
            dark: true,
            vars: vec![("--background".into(), "oklch(0.1 0 0)".into())],
        });
        let dark = all_themes()
            .into_iter()
            .find(|t| t.id == "dark")
            .expect("dark theme");
        assert_eq!(dark.label, "Dark v2");
        assert_eq!(
            all_themes().iter().filter(|t| t.id == "dark").count(),
            1,
            "a registered theme that matches a built-in id must replace it"
        );
    }

    #[test]
    fn empty_id_is_reported_not_dropped_silently() {
        let registration = register_theme(ferris_core::themes::ThemeSpec {
            id: String::new(),
            label: "Bogus".into(),
            dark: false,
            vars: vec![],
        });
        assert!(
            registration.warnings.iter().any(|w| w.contains("empty id")),
            "{:?}",
            registration.warnings
        );
        assert!(!all_themes().iter().any(|t| t.label == "Bogus"));
    }

    /// The Desk deserializes `/api/themes` into the contract type, so every theme
    /// the registry hands out must survive that round trip with its id, label and
    /// palette intact. A field rename in either crate breaks here rather than in
    /// the browser.
    #[test]
    fn every_registered_theme_survives_the_contract_round_trip() {
        let _ = register_theme(ferris_core::themes::ThemeSpec {
            id: "round-trip".into(),
            label: "Round Trip".into(),
            dark: false,
            vars: vec![("--background".into(), "oklch(0.5 0 0)".into())],
        });
        for theme in ferris_core::themes::registered_themes() {
            let json = serde_json::to_value(&theme).expect("theme serializes");
            let back: ThemeSpec =
                serde_json::from_value(json).expect("theme deserializes as the contract");
            assert_eq!(back.id, theme.id);
            assert_eq!(back.label, theme.label);
            assert_eq!(back.dark, theme.dark);
            assert_eq!(back.vars.len(), theme.vars.len());
        }
    }
}
