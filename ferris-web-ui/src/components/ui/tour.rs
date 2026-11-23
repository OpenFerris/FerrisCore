//! Guided form tours (Frappe's Form Tours).
//!
//! A tour is metadata: a list of [`TourStep`]s, each naming a field selector and
//! the copy to show. The Desk supplies the steps (from the stored `Form Tour`
//! rows) and this component owns the overlay, the target highlight and the
//! next/previous navigation. It deliberately has no data access, so a plugin
//! page or a settings screen can run a tour too.
//!
//! The highlight is a Tailwind ring added to the target element, not a
//! positioned copy of it: a copy would drift on scroll, and a rectangle drawn
//! with inline coordinates would violate the project's no-inline-style rule.
//! The popover picks a fixed corner from the step's `position`, which keeps the
//! component layout-only and themeable.

use leptos::prelude::*;

use super::button::{Button, ButtonSize, ButtonVariant};
use super::card::{Card, CardContent, CardHeader, CardTitle};

/// One step of a tour.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TourStep {
    /// CSS selector of the element to highlight (normally
    /// `[data-fieldname="status"]`).
    pub selector: String,
    pub title: String,
    pub description: String,
    /// `bottom` (default), `top`, `left` or `right`.
    pub position: String,
}

impl TourStep {
    #[must_use]
    pub fn new(selector: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            title: title.into(),
            description: String::new(),
            position: "bottom".into(),
        }
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    #[must_use]
    pub fn position(mut self, position: impl Into<String>) -> Self {
        self.position = position.into();
        self
    }
}

/// Where the popover sits for one step.
#[must_use]
pub fn popover_classes(position: &str) -> &'static str {
    match position.trim().to_ascii_lowercase().as_str() {
        "top" => "fixed inset-x-0 top-4 z-[120] flex justify-center px-4",
        "left" => "fixed inset-y-0 left-4 z-[120] flex items-center px-4",
        "right" => "fixed inset-y-0 right-4 z-[120] flex items-center px-4",
        _ => "fixed inset-x-0 bottom-4 z-[120] flex justify-center px-4",
    }
}

/// The tour overlay.
///
/// The single effect below owns both edges: when the tour closes it removes the
/// ring from the previous target, and when it opens or advances it rings the
/// new one and scrolls it into view. Keeping both in one place is what makes a
/// fast Next/Previous sequence unable to leave a stale ring behind.
#[component]
#[must_use]
#[allow(clippy::too_many_lines)] // one region: imperative DOM highlight + declarative overlay
pub fn Tour(
    /// Steps, in order. A change resets the tour to the first step.
    steps: Signal<Vec<TourStep>>,
    /// Whether the tour is showing. The component writes `false` on finish or
    /// when the list runs out.
    open: RwSignal<bool>,
    /// Called once when the tour finishes or is dismissed.
    #[prop(optional)]
    on_finish: Option<Callback<()>>,
) -> impl IntoView {
    let index = RwSignal::new(0_usize);
    let previous_selector = RwSignal::new(String::new());
    let was_open = RwSignal::new(false);

    // Reset when the tour opens; keep the current step while it stays open so a
    // parent re-render does not throw the user back to step one.
    Effect::new(move |_| {
        let now = open.get();
        if now && !was_open.get_untracked() {
            index.set(0);
        }
        was_open.set(now);
    });

    // One effect owns the target highlight: remove the ring from the step that
    // was showing, then add it to the new one and scroll it into view.
    Effect::new(move |_| {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;

            let Some(document) = web_sys::window().and_then(|window| window.document()) else {
                return;
            };
            // Clear the previous target.
            let previous = previous_selector.get_untracked();
            if !previous.is_empty() {
                if let Ok(Some(element)) = document.query_selector(&previous) {
                    clear_highlight(&element);
                }
                previous_selector.set(String::new());
            }
            if !open.get() {
                return;
            }
            let current = steps.get();
            let Some(step) = current.get(index.get()) else {
                open.set(false);
                return;
            };
            if step.selector.is_empty() {
                return;
            }
            match document.query_selector(&step.selector) {
                Ok(Some(element)) => {
                    let class_list = element.class_list();
                    let _ = class_list.add_1("ring-4");
                    let _ = class_list.add_1("ring-ring");
                    let _ = class_list.add_1("ring-offset-2");
                    let _ = class_list.add_1("rounded-md");
                    element.scroll_into_view();
                    previous_selector.set(step.selector.clone());
                    if let Some(html) = element.dyn_ref::<web_sys::HtmlElement>() {
                        let _ = html.focus();
                    }
                }
                // A selector that matches nothing is a metadata defect, not a
                // reason to freeze the tour: the copy still renders and the
                // user can move on.
                Ok(None) => previous_selector.set(step.selector.clone()),
                Err(_) => previous_selector.set(String::new()),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (&previous_selector, &steps);
        }
    });

    let total = move || steps.get().len();
    let finish = move || {
        open.set(false);
        if let Some(callback) = on_finish {
            callback.run(());
        }
    };

    view! {
        <Show when=move || open.get() && !steps.get().is_empty()>
            {move || {
                let current = steps.get();
                let step = current.get(index.get()).cloned().unwrap_or_default();
                let position = popover_classes(&step.position);
                let title = step.title.clone();
                let description = step.description.clone();
                let has_description = !description.is_empty();
                let last = index.get() + 1 >= current.len();
                view! {
                    <div
                        class=position
                        role="dialog"
                        aria-modal="false"
                        aria-label=crate::i18n::t("ui.tour.label")
                    >
                        <Card class="w-full max-w-md shadow-lg">
                            <CardHeader>
                                <p class="text-xs font-medium text-muted-foreground">
                                    {move || {
                                        let current = (index.get() + 1).to_string();
                                        let total = total().to_string();
                                        crate::i18n::t_with(
                                            "ui.tour.step",
                                            &[
                                                ("current", current.as_str()),
                                                ("total", total.as_str()),
                                            ],
                                        )
                                    }}
                                </p>
                                <CardTitle>{title}</CardTitle>
                            </CardHeader>
                            <CardContent>
                                <Show when=move || has_description>
                                    <p class="text-sm text-muted-foreground">{description.clone()}</p>
                                </Show>
                                <div class="mt-4 flex items-center justify-between gap-2">
                                    <Button
                                        variant=ButtonVariant::Ghost
                                        size=ButtonSize::Sm
                                        attr:disabled=move || index.get() == 0
                                        on:click=move |_| index.update(|i| *i = i.saturating_sub(1))
                                    >
                                        {crate::i18n::t("ui.tour.previous")}
                                    </Button>
                                    <Button
                                        variant=ButtonVariant::Default
                                        size=ButtonSize::Sm
                                        on:click=move |_| {
                                            if last {
                                                finish();
                                            } else {
                                                index.update(|i| *i += 1);
                                            }
                                        }
                                    >
                                        {if last {
                                            crate::i18n::t("ui.tour.done")
                                        } else {
                                            crate::i18n::t("ui.tour.next")
                                        }}
                                    </Button>
                                </div>
                            </CardContent>
                        </Card>
                    </div>
                }
            }}
        </Show>
    }
}

#[cfg(target_arch = "wasm32")]
fn clear_highlight(element: &web_sys::Element) {
    let class_list = element.class_list();
    let _ = class_list.remove_1("ring-4");
    let _ = class_list.remove_1("ring-ring");
    let _ = class_list.remove_1("ring-offset-2");
    let _ = class_list.remove_1("rounded-md");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_position_has_a_fixed_corner() {
        for (position, expected) in [
            ("bottom", "bottom-4"),
            ("top", "top-4"),
            ("left", "left-4"),
            ("right", "right-4"),
        ] {
            assert!(
                popover_classes(position).contains(expected),
                "{position} -> {}",
                popover_classes(position)
            );
        }
        // An unknown position falls back to the default instead of producing no
        // placement at all.
        assert_eq!(popover_classes("diagonal"), popover_classes("bottom"));
    }

    #[test]
    fn a_step_is_built_with_the_canonical_position() {
        let step =
            TourStep::new("[data-fieldname=\"status\"]", "Estado").description("Elige el estado.");
        assert_eq!(step.position, "bottom");
        assert_eq!(step.title, "Estado");
    }
}
