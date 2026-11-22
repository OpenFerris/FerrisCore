//! Theme selection and palette serialisation, shared by the app and the UI leaf.
//!
//! The Desk applies a theme by writing `data-theme` on `<html>` and injecting a
//! `<style>` with the palette's custom properties. The *policy* of which id is
//! active — the persisted id, the default, "the other" one for a toggle — is
//! pure and lives here; the DOM write lives in `ferris-web-ui` and
//! `ferris_web::theme` calls it too, so there is exactly one implementation.
//!
//! The storage key is the same one the Desk reads on boot (`ferris.theme`).
//! A toggle that wrote a different key would appear to work until the next
//! reload, which is the bug this module exists to prevent.

pub use ferris_schema::contract::theme::{ThemeSpec, builtin_themes};

/// `localStorage` key for the persisted theme id.
pub const THEME_STORAGE_KEY: &str = "ferris.theme";

/// Default theme used when nothing has been persisted, or the stored id is gone.
pub const DEFAULT_THEME: &str = "light";

/// The theme to paint, from the catalogue and the persisted id.
///
/// An unknown id (an app theme that was uninstalled, a hand-edited storage
/// value) falls back to [`DEFAULT_THEME`], and then to the first theme the
/// catalogue offers, so a caller never gets an id the picker cannot show.
#[must_use]
pub fn resolve_theme(themes: &[ThemeSpec], stored: Option<&str>) -> Option<ThemeSpec> {
    stored
        .and_then(|id| themes.iter().find(|theme| theme.id == id))
        .or_else(|| themes.iter().find(|theme| theme.id == DEFAULT_THEME))
        .or_else(|| themes.first())
        .cloned()
}

/// Whether the active theme is a dark one.
///
/// `ThemeSpec::dark` is the source of truth: a registered theme may call itself
/// `noche` and still be dark, so comparing ids would miss it. An id outside the
/// catalogue keeps the historical `dark` id as a fallback.
#[must_use]
pub fn is_dark_theme(themes: &[ThemeSpec], id: &str) -> bool {
    themes
        .iter()
        .find(|theme| theme.id == id)
        .map_or(id == "dark", |theme| theme.dark)
}

/// The id a light/dark toggle should switch to.
///
/// Looks for the first theme whose `dark` flag differs from the active one,
/// preferring the built-in ids so a catalogue with several dark themes still
/// toggles to `dark` rather than to whichever was registered last.
#[must_use]
pub fn toggled_theme_id(themes: &[ThemeSpec], id: &str) -> Option<String> {
    let dark = is_dark_theme(themes, id);
    let preferred = if dark { "light" } else { "dark" };
    themes
        .iter()
        .find(|theme| theme.id == preferred && theme.dark != dark)
        .or_else(|| themes.iter().find(|theme| theme.dark != dark))
        .map(|theme| theme.id.clone())
}

/// The CSS the injected `<style id="ferris-theme">` carries.
///
/// One `:root` block with the palette's custom properties. Values are written
/// as-is: they come from the server's registered theme or from the built-in
/// table, not from a document field, and a theme that could inject CSS would
/// already own the page.
#[must_use]
pub fn theme_css(spec: &ThemeSpec) -> String {
    let mut css = String::from(":root{");
    for (key, value) in &spec.vars {
        let key = key.trim_start_matches("--");
        css.push_str("--");
        css.push_str(key);
        css.push(':');
        css.push_str(value);
        css.push(';');
    }
    css.push('}');
    css
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_THEME, builtin_themes, is_dark_theme, resolve_theme, theme_css, toggled_theme_id,
    };

    #[test]
    fn unknown_or_missing_storage_falls_back_to_light() {
        let themes = builtin_themes();
        assert_eq!(
            resolve_theme(&themes, Some("removed-app-theme"))
                .map(|theme| theme.id)
                .as_deref(),
            Some(DEFAULT_THEME)
        );
        assert_eq!(
            resolve_theme(&themes, None)
                .map(|theme| theme.id)
                .as_deref(),
            Some(DEFAULT_THEME)
        );
    }

    #[test]
    fn a_stored_theme_wins_over_the_default() {
        let themes = builtin_themes();
        let resolved = resolve_theme(&themes, Some("dark"));
        assert_eq!(
            resolved.as_ref().map(|theme| theme.id.as_str()),
            Some("dark")
        );
        assert!(resolved.is_some_and(|theme| theme.dark));
    }

    #[test]
    fn toggling_switches_between_the_builtins() {
        let themes = builtin_themes();
        let dark = toggled_theme_id(&themes, "light");
        assert_eq!(dark.as_deref(), Some("dark"));
        assert_eq!(toggled_theme_id(&themes, "dark").as_deref(), Some("light"));
        assert!(is_dark_theme(&themes, "dark"));
        assert!(!is_dark_theme(&themes, "light"));
    }

    #[test]
    fn css_carries_the_palette_without_the_prefix() {
        let light = builtin_themes()
            .into_iter()
            .find(|theme| theme.id == "light")
            .expect("light is built in");
        let css = theme_css(&light);
        assert!(css.starts_with(":root{--background:"));
        assert!(css.contains("--foreground:"));
        assert!(!css.contains("----"));
        assert!(css.ends_with('}'));
    }
}
