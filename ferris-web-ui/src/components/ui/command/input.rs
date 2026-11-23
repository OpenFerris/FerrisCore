use super::*;

#[component]
pub fn CommandInput(
    #[prop(into, optional)] class: String,
    /// Accessible name for the search field. The palette is generic; the caller
    /// knows what it searches.
    #[prop(into, optional, default = crate::i18n::t("ui.command.search"))]
    aria_label: String,
    /// Id of the listbox the input controls, when the palette declares one.
    #[prop(into, optional)]
    aria_controls: Option<String>,
    #[prop(into, optional)] placeholder: String,
    /// Callback fired when search input changes. Use for server-side search.
    #[prop(optional)]
    on_search_change: Option<Callback<String>>,
) -> impl IntoView {
    let command_context = expect_context::<CommandContext>();
    // `shrink-0` keeps the field visible when the dialog is viewport-capped;
    // the 16px base size prevents iOS Safari from zooming the page on focus.
    let merged_class = tw_merge!(
        "ferris-command__input flex py-3 w-full h-10 min-w-0 shrink-0 text-base md:text-sm rounded-md disabled:opacity-50 disabled:cursor-not-allowed outline-hidden",
        class
    );

    view! {
        <input
            data-name="CommandInput"
            class=merged_class
            autocomplete="off"
            // TODO. Leptos does not seem to have autocorrect in keys.rs
            // autocorrect="off"
            spellcheck="false"
            aria-autocomplete="list"
            role="combobox"
            aria-expanded="true"
            aria-controls=aria_controls
            aria-label=aria_label
            placeholder=placeholder
            type="text"
            prop:value=move || command_context.search_query_signal.get()
            on:input=move |ev| {
                let value = event_target_value(&ev);
                command_context.search_query_signal.set(value.clone());
                if let Some(callback) = on_search_change {
                    callback.run(value);
                }
            }
            autofocus="true"
            // Prevent password managers from showing up
            data-1p-ignore="true"
            data-bwignore="true"
            data-lpignore="true"
        />
    }
}

#[component]
pub fn CommandEmpty(children: Children, #[prop(optional, into)] class: String) -> impl IntoView {
    let merged_class = tw_merge!("ferris-command__empty py-6 text-sm text-center", class);

    view! {
        <div data-name="CommandEmpty" class=merged_class>
            {children()}
        </div>
    }
}
