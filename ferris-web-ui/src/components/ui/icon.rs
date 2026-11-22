//! Icon names -> [`IconType`], and the [`Icon`] wrapper around `LeptosIcon`.
//!
//! Sidebar links and top-bar actions carry an icon *name* instead of an icon:
//! apps declare them in `describe()` (`"Wrench"`, `"BookOpen"`) and the desk
//! hard-codes its own chrome (`"RefreshCw"`, `"Ellipsis"`). The name is the
//! `IconType` variant spelled exactly; [`icon_for`] accepts other
//! capitalisations and separator styles as a fallback, so a name written
//! `"book-open"` instead of `"BookOpen"` still resolves instead of rendering
//! nothing, and a name that matches no icon falls back to a placeholder.

use std::{borrow::Cow, str::FromStr};

use icons::common::IconType;
use icons::icon_component::LeptosIcon;
use leptos::prelude::*;

/// Shown when a name matches no known icon: a wrong name must leave a
/// visible marker instead of silently blanking the row or the button.
const PLACEHOLDER: IconType = IconType::Circle;

/// Resolve an icon name to the icon it names.
///
/// The exact name wins; the re-cased spellings below only run as a fallback
/// for names written kebab/snake/camel style.
pub fn icon_for(name: &str) -> IconType {
    lookup(name).unwrap_or(PLACEHOLDER)
}

fn lookup(name: &str) -> Option<IconType> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    exact(name)
        .or_else(|| exact(&pascal_case(name)))
        .or_else(|| exact(&pascal_case(&name.to_lowercase())))
}

fn exact(name: &str) -> Option<IconType> {
    IconType::from_str(name).ok()
}

/// `"book-open"`, `"book_open"`, `"bookOpen"` -> `"BookOpen"`: separators
/// drop out and every segment starts uppercase, the rest is kept as written.
fn pascal_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut start_of_word = true;
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() {
            start_of_word = true;
        } else if start_of_word {
            out.extend(ch.to_uppercase());
            start_of_word = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// An icon name in the shapes the desk holds them: a plain name or an
/// `Option` — `None` renders nothing at all.
#[derive(Clone, Default)]
pub struct IconKey(Option<String>);

impl From<&str> for IconKey {
    fn from(name: &str) -> Self {
        Self(Some(name.to_string()))
    }
}

impl From<String> for IconKey {
    fn from(name: String) -> Self {
        Self(Some(name))
    }
}

impl From<Cow<'static, str>> for IconKey {
    fn from(name: Cow<'static, str>) -> Self {
        Self(Some(name.into_owned()))
    }
}

impl<T: Into<String>> From<Option<T>> for IconKey {
    fn from(name: Option<T>) -> Self {
        Self(name.map(Into::into))
    }
}

/// `LeptosIcon` keyed by name: takes a plain name or an `Option`, resolves it
/// through [`icon_for`], and renders nothing when the name is absent.
#[component]
pub fn Icon(
    #[prop(into)] name: IconKey,
    #[prop(into, optional)] class: Signal<String>,
) -> Option<AnyView> {
    name.0
        .map(|name| view! { <LeptosIcon icon=icon_for(&name) class=class.get() /> }.into_any())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_names_resolve() {
        assert_eq!(icon_for("LayoutDashboard"), IconType::LayoutDashboard);
        assert_eq!(icon_for("BookOpen"), IconType::BookOpen);
        assert_eq!(icon_for("SquareCheck"), IconType::SquareCheck);
        assert_eq!(icon_for("RefreshCw"), IconType::RefreshCw);
    }

    #[test]
    fn other_capitalisations_and_separators_fall_back() {
        assert_eq!(icon_for("book-open"), IconType::BookOpen);
        assert_eq!(icon_for("file_text"), IconType::FileText);
        assert_eq!(icon_for("briefcase"), IconType::Briefcase);
        assert_eq!(icon_for("trash-2"), IconType::Trash2);
        assert_eq!(icon_for("bookOpen"), IconType::BookOpen);
        assert_eq!(icon_for("BOOK-OPEN"), IconType::BookOpen);
        assert_eq!(icon_for("  sheet "), IconType::Sheet);
    }

    #[test]
    fn unknown_names_fall_back_to_the_placeholder() {
        assert_eq!(icon_for(""), PLACEHOLDER);
        assert_eq!(icon_for("   "), PLACEHOLDER);
        assert_eq!(icon_for("CheckSquare"), PLACEHOLDER);
        assert_eq!(icon_for("progress-report"), PLACEHOLDER);
    }
}
