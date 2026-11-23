//! Locale helpers for the shared UI primitives.
//!
//! The catalogue type is `ferris_web_shared::i18n::DeskI18n`; this module only
//! adds the Leptos lookup. A primitive that ships a built-in label, placeholder
//! or `aria-label` translates it here, so the same component is localized in
//! the Desk without taking a `label` prop for text it already owns.

use ferris_web_shared::i18n::DeskI18n;
use leptos::prelude::*;

/// A `Copy` handle to the catalogue of the current component tree.
///
/// `DeskI18n` owns a map, so a view with several closures would move it into
/// the first one. This handle is an arena id and can be captured everywhere.
#[derive(Clone, Copy)]
pub struct I18n {
    inner: leptos::prelude::StoredValue<DeskI18n>,
}

impl I18n {
    /// Translate one key.
    #[must_use]
    pub fn t(&self, key: &str) -> String {
        self.inner.with_value(|i18n| i18n.t(key))
    }

    /// Translate and interpolate `{name}` placeholders.
    #[must_use]
    pub fn t_with(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.inner.with_value(|i18n| i18n.t_with(key, args))
    }

    /// Borrow the catalogue for a closure.
    pub fn with_value<R>(&self, f: impl FnOnce(&DeskI18n) -> R) -> R {
        self.inner.with_value(f)
    }

    /// Like [`Self::with_value`], but `None` when the arena no longer owns the
    /// entry (a component torn down while an async task still holds the handle).
    pub fn try_with_value<R>(&self, f: impl FnOnce(&DeskI18n) -> R) -> Option<R> {
        self.inner.try_with_value(f)
    }

    /// An owned snapshot, for functions that take `&DeskI18n`.
    #[must_use]
    pub fn value(&self) -> DeskI18n {
        self.inner.with_value(Clone::clone)
    }
}

/// The context handle for the current component tree.
#[must_use]
pub fn use_i18n() -> I18n {
    I18n {
        inner: leptos::prelude::StoredValue::new(
            leptos::context::use_context::<DeskI18n>().unwrap_or_default(),
        ),
    }
}

/// Translate a key without allocating a stored handle (attribute defaults,
/// one-off labels).
#[must_use]
pub fn t(key: &str) -> String {
    leptos::context::use_context::<DeskI18n>().map_or_else(|| key.to_string(), |i18n| i18n.t(key))
}

/// Translate and interpolate without allocating a stored handle.
#[must_use]
pub fn t_with(key: &str, args: &[(&str, &str)]) -> String {
    leptos::context::use_context::<DeskI18n>()
        .map_or_else(|| key.to_string(), |i18n| i18n.t_with(key, args))
}
