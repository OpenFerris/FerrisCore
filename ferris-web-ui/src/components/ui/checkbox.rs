use icons::Check;
use leptos::prelude::*;
use tw_merge::tw_merge;

use crate::components::hooks::use_unique_id::use_unique_id_for;

#[component]
pub fn Checkbox(
    #[prop(into, optional)] class: String,
    #[prop(into, optional)] checked: Signal<bool>,
    #[prop(into, optional)] disabled: Signal<bool>,
    #[prop(into, optional)] on_checked_change: Option<Callback<bool>>,
    #[prop(into, optional, default = crate::i18n::t("ui.checkbox.label"))] aria_label: String,
    /// Explicit DOM id; a document-unique one is generated when omitted.
    #[prop(into, optional)]
    id: String,
    /// Id of a visible element that names the control (a `<label>` or text).
    #[prop(into, optional)]
    aria_labelledby: String,
    /// Reactive invalid state, so the form's error reaches the actual control
    /// (a `role="checkbox"` button, which a `<label>` cannot describe).
    #[prop(optional, into)]
    aria_invalid: Option<Signal<bool>>,
    /// Id of the element that explains the control (its error/description).
    #[prop(into, optional)]
    aria_describedby: Option<String>,
    /// Whether the field is required; exposed as `aria-required`.
    #[prop(optional)]
    aria_required: bool,
) -> impl IntoView {
    // The rendered control is a `<button role="checkbox">`, so a `<label for>`
    // cannot attach to it; the id is what lets a caller point a label (or its
    // own `aria-labelledby`) at this instance.
    let generated_id = use_unique_id_for("checkbox");
    let id = if id.is_empty() { generated_id } else { id };
    let labelledby = (!aria_labelledby.is_empty()).then_some(aria_labelledby);

    let checked_state = move || {
        if checked.get() {
            "checked"
        } else {
            "unchecked"
        }
    };

    // The box grows to 20px on phones and carries an invisible 10px ring
    // (`after`) so the tap target is ~40px, while the desktop box keeps its
    // 16px size and exact appearance.
    let checkbox_class = tw_merge!(
        "ferris-checkbox peer relative size-4 shrink-0 rounded-[4px] border shadow-xs transition-shadow outline-none disabled:cursor-not-allowed disabled:opacity-50 max-md:size-5 max-md:after:absolute max-md:after:-inset-2.5 max-md:after:content-['']",
        class
    );

    view! {
        <button
            data-name="Checkbox"
            class=checkbox_class
            id=id
            data-state=checked_state
            type="button"
            role="checkbox"
            aria-checked=move || checked.get().to_string()
            aria-label=aria_label
            aria-labelledby=labelledby
            aria-invalid=move || aria_invalid.map(|invalid| invalid.get().to_string())
            aria-describedby=aria_describedby
            aria-required=aria_required
            disabled=move || disabled.get()
            on:click=move |_| {
                if !disabled.get() && let Some(callback) = on_checked_change {
                    callback.run(!checked.get());
                }
            }
        >
            <span data-name="CheckboxIndicator" class="flex justify-center items-center text-current transition-none">
                {move || { checked.get().then(|| view! { <Check class="size-3.5" /> }) }}
            </span>
        </button>
    }
}
