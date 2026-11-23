use leptos::prelude::*;
use leptos_ui::clx;
use tw_merge::tw_merge;

mod components {
    use super::*;
    clx! {SwitchLabel, span, "text-sm font-medium"}
}

pub use components::*;

#[component]
pub fn Switch(
    #[prop(optional, into)] id: String,
    #[prop(optional, default = false)] checked: bool,
    #[prop(into, optional, default = crate::i18n::t("ui.switch.label"))] aria_label: String,
    #[prop(into, optional)] class: String,
) -> impl IntoView {
    let is_checked = RwSignal::new(checked);
    let state = move || {
        if is_checked.get() {
            "checked"
        } else {
            "unchecked"
        }
    };

    // The track must keep its 44x24 shape (the thumb travel is a fixed
    // `translate-x-5`), so the touch target is extended vertically by an
    // invisible pseudo-element instead of resizing the visual.
    let track_class = tw_merge!(
        "ferris-switch relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors disabled:cursor-not-allowed disabled:opacity-50 max-md:after:absolute max-md:after:-inset-y-2.5 max-md:after:inset-x-0 max-md:after:content-['']",
        class
    );

    view! {
        <button
            data-name="Switch"
            id=id
            type="button"
            role="switch"
            aria-checked=move || is_checked.get().to_string()
            aria-label=aria_label
            data-state=state
            class=track_class
            on:click=move |_| is_checked.update(|c| *c = !*c)
        >
            <span
                data-state=state
                class="ferris-switch__thumb block rounded-full ring-0 shadow-lg transition-transform pointer-events-none size-5 data-[state=checked]:translate-x-5 data-[state=unchecked]:translate-x-0"
            />
        </button>
    }
}
