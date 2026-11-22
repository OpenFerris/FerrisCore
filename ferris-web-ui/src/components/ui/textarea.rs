use leptos::html;
use leptos::prelude::*;
use tw_merge::tw_merge;

#[component]
pub fn Textarea(
    // Styling
    #[prop(into, optional)] class: String,

    // Common HTML attributes
    #[prop(into, optional)] placeholder: Option<String>,
    #[prop(into, optional)] name: Option<String>,
    #[prop(into, optional)] id: Option<String>,
    #[prop(optional)] disabled: bool,
    #[prop(optional)] readonly: bool,
    #[prop(optional)] required: bool,
    #[prop(optional)] autofocus: bool,
    #[prop(into, optional)] rows: Option<u32>,
    #[prop(optional, into)] aria_invalid: Option<Signal<bool>>,
    #[prop(into, optional)] aria_describedby: Option<String>,

    // Two-way binding (like bind:value)
    #[prop(into, optional)] bind_value: Option<RwSignal<String>>,

    // Ref for direct DOM access
    #[prop(optional)] node_ref: NodeRef<html::Textarea>,
) -> impl IntoView {
    let merged_class = tw_merge!(
        "ferris-textarea flex field-sizing-content min-h-16 w-full min-w-0 rounded-md border px-3 py-2 text-base shadow-xs transition-[color,box-shadow] outline-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm",
        class
    );

    match bind_value {
        Some(signal) => view! {
            <textarea
                data-name="Textarea"
                class=merged_class
                placeholder=placeholder
                name=name
                id=id
                disabled=disabled
                readonly=readonly
                required=required
                autofocus=autofocus
                rows=rows
                aria-invalid=move || {
                    aria_invalid
                        .map(|state| state.get().to_string())
                        .unwrap_or_else(|| "false".to_string())
                }
                aria-describedby=aria_describedby
                bind:value=signal
                node_ref=node_ref
            />
        }
        .into_any(),
        None => view! {
            <textarea
                data-name="Textarea"
                class=merged_class
                placeholder=placeholder
                name=name
                id=id
                disabled=disabled
                readonly=readonly
                required=required
                autofocus=autofocus
                rows=rows
                aria-invalid=move || {
                    aria_invalid
                        .map(|state| state.get().to_string())
                        .unwrap_or_else(|| "false".to_string())
                }
                aria-describedby=aria_describedby
                node_ref=node_ref
            />
        }
        .into_any(),
    }
}
