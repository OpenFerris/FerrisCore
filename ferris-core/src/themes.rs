//! Desk themes: a small, extensible registry of *app-registered* UI themes.
//!
//! The two built-in themes (`light` and `dark`) are **not** defined here: their
//! palette is the wire contract's
//! `ferris_schema::contract::theme::builtin_themes`, the one Rust definition
//! shared by the native server and the wasm `hydrate` client. This crate sits
//! *below* `ferris-schema` and cannot depend on it, so it owns only the extra
//! themes an app adds: either call [`register_theme`] / [`register_themes`]
//! once at startup with a [`ThemeSpec`], or declare a `"themes": [...]` array in
//! the plugin's `describe()` JSON — the framework registers them when the
//! plugin is loaded. `GET /api/themes` (`ferris-api`) composes the built-ins
//! with [`registered_themes`].
//!
//! The palette keys are the CSS custom properties the UI consumes (`--background`,
//! `--foreground`, `--muted`, `--accent`, `--border`, …). The built-in values
//! mirror `ferris-web/style/tailwind.css`'s `:root` / `.dark` blocks; the
//! `design_system` test keeps the contract palette and the stylesheet in sync.
//!
//! The single Rust definition of that palette is
//! `ferris_schema::contract::theme::builtin_themes`; it is reachable from the
//! wasm `hydrate` build, which this crate is deliberately absent from.

use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

/// One theme: a palette of CSS custom properties to apply to `:root`.
///
/// The wire contract has an identical
/// `ferris_schema::contract::theme::ThemeSpec`; the
/// `ferris-api` round-trip test keeps the two field-by-field compatible. The
/// duplication is structural only — this crate cannot depend on `ferris-schema`
/// (that would be a cycle) — and carries no palette values.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeSpec {
    /// Machine id, e.g. `"dark"`. Must be unique across registered themes.
    pub id: String,
    /// Human-readable name shown in the picker, e.g. `"Oscuro"`.
    pub label: String,
    /// Whether this is a dark theme (drives icons and `color-scheme`).
    pub dark: bool,
    /// Ordered CSS custom properties to set on `:root`, e.g.
    /// `[("--background", "oklch(1 0 0)"), ...]`.
    #[serde(default)]
    pub vars: Vec<(String, String)>,
}

/// Outcome of a theme registration: the ids that were kept, in registration
/// order, plus any validation warnings the caller should surface.
///
/// Mirrors [`crate::ui_extensions::Registration`] / the workspace registration
/// value: warnings describe declarations that were dropped, the registration
/// itself always succeeds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registration {
    /// Ids of the themes that were registered (or replaced).
    pub names: Vec<String>,
    /// Human-readable validation messages (empty id dropped, …).
    pub warnings: Vec<String>,
}

/// Apps registered extra themes (beyond the two built-ins).
static EXTRA_THEMES: OnceLock<RwLock<Vec<ThemeSpec>>> = OnceLock::new();

fn registry() -> &'static RwLock<Vec<ThemeSpec>> {
    EXTRA_THEMES.get_or_init(|| RwLock::new(Vec::new()))
}

/// Register one extra theme.
///
/// Returns the id that was kept, or a warning when it was dropped. A theme
/// with an empty id is reported in [`Registration::warnings`]; a theme with the
/// same id replaces the previous registration (last write wins).
#[must_use]
pub fn register_theme(spec: ThemeSpec) -> Registration {
    if spec.id.trim().is_empty() {
        return Registration {
            names: Vec::new(),
            warnings: vec!["theme: empty id — dropped".to_string()],
        };
    }
    let id = spec.id.clone();
    {
        let mut themes = registry()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = themes.iter_mut().find(|t| t.id == spec.id) {
            *existing = spec;
        } else {
            themes.push(spec);
        }
    }
    Registration {
        names: vec![id],
        warnings: Vec::new(),
    }
}

/// Register multiple extra themes at once, aggregating their ids and warnings.
#[must_use]
pub fn register_themes(specs: Vec<ThemeSpec>) -> Registration {
    let mut out = Registration::default();
    for spec in specs {
        let registration = register_theme(spec);
        out.names.extend(registration.names);
        out.warnings.extend(registration.warnings);
    }
    out
}

/// Snapshot of the app-registered themes, in registration order.
///
/// The two built-ins are not included — the caller composes them from the wire
/// contract's `builtin_themes`; this returns only the extras.
#[must_use]
pub fn registered_themes() -> Vec<ThemeSpec> {
    registry()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_theme_shows_up() {
        let registration = register_theme(ThemeSpec {
            id: "solarized".into(),
            label: "Solarized".into(),
            dark: false,
            vars: vec![("--background".into(), "oklch(0.985 0.03 80)".into())],
        });
        assert_eq!(registration.names, vec!["solarized".to_string()]);
        assert!(registration.warnings.is_empty());
        assert!(registered_themes().iter().any(|t| t.id == "solarized"));
    }

    #[test]
    fn re_registration_replaces() {
        let _ = register_theme(ThemeSpec {
            id: "twilight".into(),
            label: "Twilight".into(),
            dark: true,
            vars: vec![("--background".into(), "oklch(0.1 0 0)".into())],
        });
        let _ = register_theme(ThemeSpec {
            id: "twilight".into(),
            label: "Twilight v2".into(),
            dark: true,
            vars: vec![("--background".into(), "oklch(0.12 0 0)".into())],
        });
        let matches: Vec<_> = registered_themes()
            .into_iter()
            .filter(|t| t.id == "twilight")
            .collect();
        assert_eq!(matches.len(), 1, "a re-registration replaces in place");
        assert_eq!(matches[0].label, "Twilight v2");
        assert_eq!(
            matches[0].vars[0],
            ("--background".to_string(), "oklch(0.12 0 0)".to_string())
        );
    }

    #[test]
    fn empty_id_is_reported() {
        let registration = register_theme(ThemeSpec {
            id: String::new(),
            label: "Bogus".into(),
            dark: false,
            vars: vec![],
        });
        assert!(registration.names.is_empty());
        assert!(
            registration.warnings.iter().any(|w| w.contains("empty id")),
            "{:?}",
            registration.warnings
        );
        assert!(!registered_themes().iter().any(|t| t.label == "Bogus"));
    }

    #[test]
    fn register_themes_aggregates() {
        let registration = register_themes(vec![
            ThemeSpec {
                id: "forest".into(),
                label: "Forest".into(),
                dark: false,
                vars: vec![],
            },
            ThemeSpec {
                id: "  ".into(),
                label: "Blank".into(),
                dark: false,
                vars: vec![],
            },
        ]);
        assert_eq!(registration.names, vec!["forest".to_string()]);
        assert_eq!(registration.warnings.len(), 1);
    }

    #[test]
    fn describe_json_roundtrips() {
        let spec = ThemeSpec {
            id: "custom".into(),
            label: "Custom".into(),
            dark: false,
            vars: vec![("--background".into(), "oklch(1 0 0)".into())],
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: ThemeSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
    }
}
