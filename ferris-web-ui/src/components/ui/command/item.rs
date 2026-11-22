use super::*;

#[component]
pub fn CommandItem(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] value: String,
    #[prop(optional)] on_select: Option<Callback<()>>,
    #[prop(default = false.into(), into)] selected: Signal<bool>,
    /// Reserve space for check icon even when not selected (for alignment)
    #[prop(default = false)]
    reserve_check_space: bool,
) -> impl IntoView {
    let command_context = expect_context::<CommandContext>();
    let value_for_filter = value;

    let merged_class = tw_merge!(
        "ferris-command__item group relative flex gap-2 items-center px-2 py-1.5 text-sm rounded-sm cursor-default select-none outline-none data-[disabled=true]:pointer-events-none data-[disabled=true]:opacity-50",
        class
    );

    let is_visible = Memo::new(move |_| {
        // Skip client-side filtering when should_filter is false (server-side search)
        if !command_context.should_filter {
            return true;
        }

        let search = command_context.search_query_signal.get().to_lowercase();
        if search.is_empty() {
            return true;
        }
        value_for_filter.to_lowercase().contains(&search)
    });

    // Check icon class: always visible space when reserve_check_space, otherwise hidden when not selected
    let check_class = if reserve_check_space {
        "ml-auto size-4 ferris-command__item-check opacity-0 group-aria-selected:opacity-100"
    } else {
        "ml-auto size-4 ferris-command__item-check hidden group-aria-selected:block"
    };

    view! {
        <div
            data-name="CommandItem"
            class=merged_class
            role="option"
            tabindex="0"
            aria-selected=move || selected.get().to_string()
            style:display=move || if is_visible.get() { "flex" } else { "none" }
            on:click=move |_| {
                if let Some(callback) = on_select {
                    callback.run(());
                }
            }
            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                if ev.key() == "Enter" || ev.key() == " " {
                    ev.prevent_default();
                    if let Some(callback) = on_select {
                        callback.run(());
                    }
                }
            }
        >
            {children()}
            <Check class=check_class />
        </div>
    }
}
