use icons::ChevronDown;
use leptos::prelude::*;
use leptos_ui::clx;
use tw_merge::*;

// TODO 💪: Use Label from label.rs.

#[component]
pub fn SelectNative(
    children: Children,
    #[prop(into)] id: String,
    #[prop(optional, into)] class: String,
    #[prop(into, optional)] aria_label: Option<String>,
    #[prop(default = RwSignal::new(String::new()).read_only())] value: ReadSignal<String>,
    #[prop(optional)] on_change: Option<leptos::prelude::Callback<leptos::ev::Event>>,
    #[prop(optional, into)] aria_invalid: Option<Signal<bool>>,
    #[prop(into, optional)] aria_describedby: Option<String>,
    /// Whether the field is required; native `required` plus `aria-required`.
    #[prop(optional)]
    required: bool,
) -> impl IntoView {
    clx! {
        SelectNativeRoot, select,
        "ferris-input peer inline-flex h-9 max-md:h-10 w-full min-w-0 cursor-pointer appearance-none items-center rounded-md border pe-8 ps-3 py-1 text-base shadow-xs transition-[color,box-shadow] outline-none md:text-sm disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 has-[option[disabled]:checked]:text-muted-foreground",
    }

    view! {
        <div class="relative">
            <SelectNativeRoot
                attr:id=id
                attr:data-name="SelectNative"
                attr:aria-label=aria_label
                attr:aria-invalid=move || {
                    aria_invalid
                        .map(|state| state.get().to_string())
                        .unwrap_or_else(|| "false".to_string())
                }
                attr:aria-describedby=aria_describedby
                attr:aria-required=required
                attr:required=required
                class=class
                prop:value=move || value.get()
                on:change=move |ev| {
                    if let Some(handler) = on_change {
                        handler.run(ev);
                    }
                }
            >
                {children()}
            </SelectNativeRoot>

            <span class="flex absolute inset-y-0 justify-center items-center w-9 h-full pointer-events-none end-0 text-muted-foreground/80 peer-disabled:opacity-50 [&_svg:not([class*='size-'])]:size-4">
                <ChevronDown />
            </span>
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn OverlappingLabel(#[prop(into)] r#for: String, #[prop(into)] label: String) -> impl IntoView {
    view! {
        <label
            for=r#for
            class="block absolute top-0 z-10 px-2 text-xs font-medium -translate-y-1/2 start-1 bg-background text-foreground group-has-[select:disabled]:opacity-50"
        >
            {label}
        </label>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn LabelNative(
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] r#for: String,
    children: Children,
) -> impl IntoView {
    let class = tw_merge!(
        "text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
        class
    );

    view! {
        <label class=class r#for=r#for>
            {children()}
        </label>
    }
}
