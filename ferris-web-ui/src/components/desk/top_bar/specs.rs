//! The declarative action types a view publishes into the slot: buttons,
//! dropdowns and the selection-aware control. The top bar renders the
//! matching control for each — it knows the shapes, never the app.

use leptos::prelude::*;
use leptos_router::NavigateOptions;

use ferris_web_shared::meta::ViewSpec;

use crate::components::ui::button::ButtonVariant;

// ---------------------------------------------------------------------------
// Action types
// ---------------------------------------------------------------------------

/// One entry of a [`DropdownSpec`] menu.
#[derive(Debug, Clone)]
pub struct DropdownItem {
    pub label: String,
    pub icon: Option<&'static str>,
    pub destructive: bool,
    /// Rendered inert — no click, dimmed. A capability the backend does not
    /// have yet is shown *and labelled* rather than silently doing nothing.
    pub disabled: bool,
    /// A divider between two groups of items: no label, no action.
    pub separator: bool,
    /// When set, the item renders as a link instead of an action.
    pub href: Option<String>,
    pub on_click: Callback<()>,
}

impl DropdownItem {
    /// A plain action item (no icon, not destructive, no href).
    pub fn action(label: impl Into<String>, on_click: Callback<()>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            destructive: false,
            disabled: false,
            separator: false,
            href: None,
            on_click,
        }
    }

    /// A link item (rendered as `<a href=…>`).
    pub fn link(label: impl Into<String>, href: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            destructive: false,
            disabled: false,
            separator: false,
            href: Some(href.into()),
            on_click: Callback::new(|_| {}),
        }
    }

    /// A divider between two groups of items.
    pub fn separator() -> Self {
        Self {
            label: String::new(),
            icon: None,
            destructive: false,
            disabled: false,
            separator: true,
            href: None,
            on_click: Callback::new(|_| {}),
        }
    }

    #[must_use]
    pub fn with_icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

/// A plain dropdown: a label + a list of items. Rendered by the bar's own
/// dropdown control (`super::render`).
#[derive(Debug, Clone)]
pub struct DropdownSpec {
    pub label: String,
    /// Accessible name for an icon-only trigger (empty `label`). Falls back to
    /// `label` when it is set.
    pub aria_label: Option<String>,
    pub icon: Option<&'static str>,
    pub items: Vec<DropdownItem>,
    pub selectable: bool,
}

/// The active state of a [`SelectionMenuSpec`].
#[derive(Debug, Clone)]
pub struct SelectionState {
    pub label: String,
    pub items: Vec<DropdownItem>,
}

/// A selection-aware control: renders a plain button when nothing is selected,
/// a dropdown when there is a selection.
///
/// Both states are collapsed into a single [`Signal<Option<SelectionState>>`],
/// which makes an inconsistent state (selection on with no items, etc.)
/// unrepresentable.
#[derive(Clone)]
pub struct SelectionMenuSpec {
    /// Label shown when there is no selection (e.g. "Nuevo").
    pub idle_label: String,
    /// Icon shown on both the idle button and the active dropdown.
    pub idle_icon: Option<&'static str>,
    /// Action fired when the idle button is clicked.
    pub on_idle_click: Callback<()>,
    /// `None` → idle button; `Some(state)` → dropdown.
    pub selection: Signal<Option<SelectionState>>,
}

/// A button action in the top bar.
#[derive(Clone)]
pub struct ButtonSpec {
    pub label: String,
    /// Accessible name for an icon-only button (empty `label`). Falls back to
    /// `label` when it is set.
    pub aria_label: Option<String>,
    pub icon: Option<&'static str>,
    pub variant: ButtonVariant,
    /// Rendered inert (dimmed, not clickable) — e.g. previous/next on the first
    /// or last document.
    pub disabled: bool,
    pub on_click: Callback<()>,
}

/// The dashboard full-screen toggle.
///
/// It is its own variant, not a [`ButtonSpec`], because it is the only action
/// the bar keeps in full-screen mode (see `TopBarChromeSpec`): the bar filters
/// the slot by variant, so a dashboard cannot accidentally leave an edit
/// button on a screen that belongs to the board.
#[derive(Clone)]
pub struct FullScreenToggleSpec {
    /// Whether full screen is already active (the control flips its icon and
    /// accessible name).
    pub active: bool,
    pub on_click: Callback<()>,
}

/// A declarative top-bar action. Views publish a `Vec<TopBarAction>` and the
/// top bar renders the matching control for each variant.
#[derive(Clone)]
pub enum TopBarAction {
    Button(ButtonSpec),
    Dropdown(DropdownSpec),
    SelectionMenu(SelectionMenuSpec),
    FullScreenToggle(FullScreenToggleSpec),
}

impl TopBarAction {
    /// Build a "view switcher" dropdown from a list of view specs.
    ///
    /// Returns `None` when there's only one view (nothing to switch to).
    pub fn view_switcher(
        views: &[ViewSpec],
        current_view_id: &str,
        navigate: Callback<(String, NavigateOptions)>,
    ) -> Option<Self> {
        if views.len() <= 1 {
            return None;
        }

        let current_view = views
            .iter()
            .find(|v| v.id == current_view_id)
            .or_else(|| views.first())?;

        let items = views
            .iter()
            .filter(|v| v.id != current_view_id)
            .map(|v| {
                let route = v.route.clone();
                DropdownItem::action(
                    v.label.to_string(),
                    Callback::new(move |_| {
                        // `try_run`: this item is built by the caller's current
                        // run, so it can outlive its owner by one teardown (see
                        // the module docs of `top_bar`).
                        let _ = navigate.try_run((route.clone(), NavigateOptions::default()));
                    }),
                )
                .with_icon(v.icon)
            })
            .collect();

        Some(TopBarAction::Dropdown(DropdownSpec {
            label: current_view.label.to_string(),
            aria_label: None,
            icon: Some(current_view.icon),
            items,
            selectable: true,
        }))
    }
}
