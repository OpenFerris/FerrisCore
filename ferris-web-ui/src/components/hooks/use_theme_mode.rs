//! Light/dark switching for the components that offer it.
//!
//! The active theme belongs to the application (`ferris_web::theme::ThemeCtx`),
//! not to this leaf crate: `ferris-web-ui` cannot depend on `ferris-web`, so it
//! cannot reach those signals directly. The app therefore publishes a
//! [`ThemeController`] into context and this hook is the component-facing
//! adapter. Before the bridge, this module wrote its own `darkmode` key and
//! never touched the real palette: the toggle looked alive and changed nothing
//! after a reload.
//!
//! When no controller is present (a component preview rendered outside the
//! Desk shell) the hook falls back to the shared policy: persist
//! `ferris.theme` — the same key the app reads on boot — and paint the built-in
//! palette through [`theme_dom`], so the switch still works and still survives
//! a reload.

use leptos::prelude::*;

use super::theme_dom;

/// The application's theme engine, published by `ferris_web::theme`.
#[derive(Clone, Copy)]
pub struct ThemeController {
    /// Whether the active theme is dark (`ThemeSpec::dark`, not an id match).
    pub is_dark: Signal<bool>,
    /// Switch to a dark or light theme, applying and persisting it.
    pub set_dark: Callback<bool>,
}

#[derive(Clone, Copy)]
pub struct ThemeMode {
    state: RwSignal<bool>,
    controller: Option<ThemeController>,
}

// The key is shared with the boot path on purpose: a second key only makes
// the toggle look effective until the next reload.
//
// `ferris_web_shared::theme::THEME_STORAGE_KEY` is the one definition; this
// module reaches it through [`theme_dom`], which owns the browser write.

/// Hook to access the dark mode context
///
/// Returns the `ThemeMode` instance from context for easy access
#[must_use]
pub fn use_theme_mode() -> ThemeMode {
    expect_context::<ThemeMode>()
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

impl ThemeMode {
    #[must_use]
    /// Initializes a new `ThemeMode` instance.
    pub fn init() -> Self {
        let controller = use_context::<ThemeController>();
        let theme_mode = Self {
            state: RwSignal::new(Self::initial(controller)),
            controller,
        };

        provide_context(theme_mode);

        // Keep the local flag in step when the theme changes elsewhere (the
        // settings picker, a server refresh of the catalogue).
        if let Some(controller) = controller {
            Effect::new(move |_| theme_mode.state.set(controller.is_dark.get()));
        }

        theme_mode
    }

    pub fn toggle(&self) {
        self.set(!self.state.get_untracked());
    }

    pub fn set_dark(&self) {
        self.set(true);
    }

    pub fn set_light(&self) {
        self.set(false);
    }

    /// - `dark`: Set to `true` for dark mode, and `false` for light mode.
    pub fn set(&self, dark: bool) {
        self.state.set(dark);
        if let Some(controller) = self.controller {
            controller.set_dark.run(dark);
        } else {
            Self::apply_fallback(dark);
        }
    }

    #[must_use]
    pub fn get(&self) -> bool {
        self.state.get()
    }

    #[must_use]
    pub fn is_dark(&self) -> bool {
        self.state.get()
    }

    #[must_use]
    pub fn is_light(&self) -> bool {
        !self.state.get()
    }

    /// Initial flag: the controller, then storage, then the OS preference.
    fn initial(controller: Option<ThemeController>) -> bool {
        if let Some(controller) = controller {
            return controller.is_dark.get_untracked();
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self::get_storage_state().unwrap_or_else(Self::prefers_dark_mode)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            false
        }
    }

    /// Repaint and persist without the application controller.
    fn apply_fallback(dark: bool) {
        let themes = ferris_web_shared::theme::builtin_themes();
        let id = if dark { "dark" } else { "light" };
        theme_dom::persist_theme_id(id);
        if let Some(spec) = ferris_web_shared::theme::resolve_theme(&themes, Some(id)) {
            theme_dom::apply_theme(&spec);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn get_storage_state() -> Option<bool> {
        theme_dom::stored_theme_id().map(|id| {
            ferris_web_shared::theme::is_dark_theme(
                &ferris_web_shared::theme::builtin_themes(),
                &id,
            )
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn prefers_dark_mode() -> bool {
        window()
            .match_media("(prefers-color-scheme: dark)")
            .ok()
            .flatten()
            .map(|media| media.matches())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{ThemeController, ThemeMode};
    use leptos::prelude::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI8, Ordering};

    #[test]
    fn toggling_asks_the_application_engine_for_the_opposite_mode() {
        // Signals need a live arena even outside a component.
        let owner = Owner::new();
        owner.set();

        let requested = Arc::new(AtomicI8::new(-1));
        let sink = requested.clone();
        let mode = ThemeMode {
            state: RwSignal::new(false),
            controller: Some(ThemeController {
                is_dark: Signal::derive(|| false),
                set_dark: Callback::new(move |dark| {
                    sink.store(i8::from(dark), Ordering::Relaxed);
                }),
            }),
        };

        mode.toggle();
        assert_eq!(requested.load(Ordering::Relaxed), 1);
        assert!(mode.is_dark());

        mode.toggle();
        assert_eq!(requested.load(Ordering::Relaxed), 0);
        assert!(mode.is_light());
    }
}
