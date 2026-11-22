//! Desk themes: `GET /api/themes`.
//!
//! A theme is data — an id, a label and the CSS custom properties to apply on
//! `:root`. The *registry* (which app themes exist, who registered them) lives
//! in `ferris_core::themes`; this is only the wire shape, so the browser can
//! deserialize the feed without the core crate.
//!
//! The built-in `light`/`dark` palettes live here too. This module is compiled
//! native *and* for `wasm32-unknown-unknown` (`ferris-web` depends on this crate
//! with `default-features = false`, so `contract` is all it sees), which makes
//! it the one place the server (`ferris-api`), the Desk fallback
//! (`ferris-web::theme`) and the `design_system` test can all read the palette
//! from — without pulling `ferris-core` into the hydrate build. `ferris-core`
//! sits *below* this crate and cannot depend on it, so it may only own the
//! registry of app-registered themes.
//!
//! The values mirror `ferris-web/style/tailwind.css`'s `:root`/`.dark` blocks;
//! `ferris-web/tests/design_system.rs` fails if either side drifts.
//!
//! Deliberately both `Serialize` and `Deserialize` even though the Desk only
//! reads: that makes the type round-trippable so the server can serialize the
//! registry's `ThemeSpec` through it and both sides are checked against one
//! definition.

use serde::{Deserialize, Serialize};

/// One theme: a palette of CSS custom properties for `:root`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeSpec {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub dark: bool,
    #[serde(default)]
    pub vars: Vec<(String, String)>,
}

/// The built-in themes, in picker order: `light` then `dark`.
///
/// The single Rust definition of the built-in palette. `ferris-core` owns only
/// the app-registered extras, `ferris-api` composes both feeds for
/// `GET /api/themes`, and `ferris-web::theme` uses this list directly as its
/// offline fallback — so there is no second copy to drift.
#[must_use]
pub fn builtin_themes() -> Vec<ThemeSpec> {
    vec![light_theme(), dark_theme()]
}

/// `light` — the default / current look. Mirrors `tailwind.css`'s `:root`.
#[must_use]
pub fn light_theme() -> ThemeSpec {
    ThemeSpec {
        id: "light".into(),
        label: "Claro".into(),
        dark: false,
        vars: vec![
            ("--background", "oklch(1 0 0)"),
            ("--foreground", "oklch(0.153 0.006 107.1)"),
            ("--card", "oklch(1 0 0)"),
            ("--card-foreground", "oklch(0.153 0.006 107.1)"),
            ("--popover", "oklch(1 0 0)"),
            ("--popover-foreground", "oklch(0.153 0.006 107.1)"),
            ("--primary", "oklch(0.228 0.013 107.4)"),
            ("--primary-foreground", "oklch(0.988 0.003 106.5)"),
            ("--secondary", "oklch(0.966 0.005 106.5)"),
            ("--secondary-foreground", "oklch(0.228 0.013 107.4)"),
            ("--muted", "oklch(0.966 0.005 106.5)"),
            ("--muted-foreground", "oklch(0.58 0.031 107.3)"),
            ("--accent", "oklch(0.966 0.005 106.5)"),
            ("--accent-foreground", "oklch(0.228 0.013 107.4)"),
            ("--border", "oklch(0.93 0.007 106.5)"),
            ("--input", "oklch(0.93 0.007 106.5)"),
            ("--ring", "oklch(0.737 0.021 106.9)"),
            ("--destructive", "oklch(0.577 0.245 27.325)"),
            ("--destructive-foreground", "oklch(0.988 0.003 106.5)"),
            ("--success", "oklch(0.723 0.192 149.579)"),
            ("--success-foreground", "oklch(0.988 0.003 106.5)"),
            ("--warning", "oklch(0.795 0.184 86.047)"),
            ("--warning-foreground", "oklch(0.216 0.006 56.043)"),
            ("--info", "oklch(0.685 0.169 237.323)"),
            ("--info-foreground", "oklch(0.988 0.003 106.5)"),
            ("--surface", "oklch(0.985 0.002 106.5)"),
            ("--surface-foreground", "oklch(0.153 0.006 107.1)"),
            ("--overlay", "oklch(0 0 0 / 50%)"),
            ("--sidebar", "oklch(0.985 0.002 106.5)"),
            ("--sidebar-foreground", "oklch(0.153 0.006 107.1)"),
            ("--sidebar-border", "oklch(0.93 0.007 106.5)"),
            ("--sidebar-accent", "oklch(0.966 0.005 106.5)"),
            ("--sidebar-accent-foreground", "oklch(0.228 0.013 107.4)"),
            ("--sidebar-ring", "oklch(0.737 0.021 106.9)"),
            ("--topbar", "oklch(1 0 0)"),
            ("--topbar-foreground", "oklch(0.153 0.006 107.1)"),
            ("--topbar-border", "oklch(0.93 0.007 106.5)"),
            ("--folder-slate", "oklch(0.446 0.043 257.281)"),
            ("--folder-blue", "oklch(0.546 0.245 262.881)"),
            ("--folder-emerald", "oklch(0.596 0.145 163.225)"),
            ("--folder-amber", "oklch(0.555 0.163 48.998)"),
            ("--folder-rose", "oklch(0.586 0.253 17.585)"),
            ("--folder-violet", "oklch(0.541 0.281 293.009)"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
    }
}

/// `dark` — the built-in dark theme. Mirrors `tailwind.css`'s `.dark`.
#[must_use]
pub fn dark_theme() -> ThemeSpec {
    ThemeSpec {
        id: "dark".into(),
        label: "Oscuro".into(),
        dark: true,
        vars: vec![
            ("--background", "oklch(0.153 0.006 107.1)"),
            ("--foreground", "oklch(0.988 0.003 106.5)"),
            ("--card", "oklch(0.228 0.013 107.4)"),
            ("--card-foreground", "oklch(0.988 0.003 106.5)"),
            ("--popover", "oklch(0.228 0.013 107.4)"),
            ("--popover-foreground", "oklch(0.988 0.003 106.5)"),
            ("--primary", "oklch(0.93 0.007 106.5)"),
            ("--primary-foreground", "oklch(0.228 0.013 107.4)"),
            ("--secondary", "oklch(0.286 0.016 107.4)"),
            ("--secondary-foreground", "oklch(0.988 0.003 106.5)"),
            ("--muted", "oklch(0.286 0.016 107.4)"),
            ("--muted-foreground", "oklch(0.737 0.021 106.9)"),
            ("--accent", "oklch(0.286 0.016 107.4)"),
            ("--accent-foreground", "oklch(0.988 0.003 106.5)"),
            ("--border", "oklch(1 0 0 / 10%)"),
            ("--input", "oklch(1 0 0 / 15%)"),
            ("--ring", "oklch(0.58 0.031 107.3)"),
            ("--destructive", "oklch(0.704 0.191 22.216)"),
            ("--destructive-foreground", "oklch(0.988 0.003 106.5)"),
            ("--success", "oklch(0.696 0.17 162.48)"),
            ("--success-foreground", "oklch(0.988 0.003 106.5)"),
            ("--warning", "oklch(0.828 0.189 84.429)"),
            ("--warning-foreground", "oklch(0.216 0.006 56.043)"),
            ("--info", "oklch(0.746 0.16 232.661)"),
            ("--info-foreground", "oklch(0.216 0.006 56.043)"),
            ("--surface", "oklch(0.205 0.012 107.3)"),
            ("--surface-foreground", "oklch(0.988 0.003 106.5)"),
            ("--overlay", "oklch(0 0 0 / 60%)"),
            ("--sidebar", "oklch(0.205 0.012 107.3)"),
            ("--sidebar-foreground", "oklch(0.988 0.003 106.5)"),
            ("--sidebar-border", "oklch(1 0 0 / 10%)"),
            ("--sidebar-accent", "oklch(0.286 0.016 107.4)"),
            ("--sidebar-accent-foreground", "oklch(0.988 0.003 106.5)"),
            ("--sidebar-ring", "oklch(0.58 0.031 107.3)"),
            ("--topbar", "oklch(0.228 0.013 107.4)"),
            ("--topbar-foreground", "oklch(0.988 0.003 106.5)"),
            ("--topbar-border", "oklch(1 0 0 / 10%)"),
            ("--folder-slate", "oklch(0.869 0.022 252.894)"),
            ("--folder-blue", "oklch(0.809 0.105 251.813)"),
            ("--folder-emerald", "oklch(0.845 0.143 164.978)"),
            ("--folder-amber", "oklch(0.879 0.169 91.605)"),
            ("--folder-rose", "oklch(0.81 0.117 11.638)"),
            ("--folder-violet", "oklch(0.811 0.111 293.571)"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `id` is the only required key: it is the theme's identity and the value the
    /// Desk persists, so a theme without one is not a theme. Everything else has
    /// a default because a bare theme (no palette, no label) is still selectable.
    #[test]
    fn only_the_id_is_required() {
        let bare: ThemeSpec = serde_json::from_value(serde_json::json!({ "id": "light" }))
            .expect("a theme with only an id parses");
        assert_eq!(bare.id, "light");
        assert_eq!(bare.label, "");
        assert!(!bare.dark);
        assert!(bare.vars.is_empty());

        let missing_id =
            serde_json::from_value::<ThemeSpec>(serde_json::json!({ "label": "Oscuro" }));
        assert!(
            missing_id.is_err(),
            "a theme without an id must not deserialize"
        );
    }

    /// The palette is an ordered list of `--name: value` pairs and the Desk
    /// rewrites the `--` prefix when it injects the `<style>`, so the pairs
    /// must survive in order.
    #[test]
    fn the_palette_survives_as_ordered_pairs() {
        let spec: ThemeSpec = serde_json::from_value(serde_json::json!({
            "id": "dark",
            "dark": true,
            "vars": [["--background", "oklch(0.153 0.006 107.1)"], ["--ring", "oklch(0.58 0.031 107.3)"]],
        }))
        .expect("theme parses");

        assert_eq!(
            spec.vars,
            vec![
                (
                    "--background".to_string(),
                    "oklch(0.153 0.006 107.1)".to_string()
                ),
                ("--ring".to_string(), "oklch(0.58 0.031 107.3)".to_string()),
            ]
        );
    }

    #[test]
    fn builtins_are_light_then_dark() {
        let themes = builtin_themes();
        assert_eq!(themes.len(), 2);
        assert_eq!(themes[0].id, "light");
        assert!(!themes[0].dark);
        assert_eq!(themes[1].id, "dark");
        assert!(themes[1].dark);
        for theme in themes {
            assert!(!theme.id.is_empty());
            assert!(!theme.label.is_empty());
            assert!(
                theme.vars.iter().any(|(k, _)| k == "--background"),
                "{} must define --background",
                theme.id
            );
            assert!(
                theme.vars.iter().any(|(k, _)| k == "--foreground"),
                "{} must define --foreground",
                theme.id
            );
        }
    }

    /// A token added to one built-in but not the other would leave the missing
    /// side inheriting the static stylesheet value under runtime theming; keep
    /// the key sets identical.
    #[test]
    fn builtins_share_the_same_token_set() {
        let keys = |t: &ThemeSpec| -> Vec<String> {
            t.vars.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()
        };
        assert_eq!(keys(&light_theme()), keys(&dark_theme()));
    }

    #[test]
    fn builtins_json_roundtrip() {
        for spec in builtin_themes() {
            let json = serde_json::to_string(&spec).expect("theme serializes");
            let back: ThemeSpec = serde_json::from_str(&json).expect("theme round-trips");
            assert_eq!(spec, back);
        }
    }
}
