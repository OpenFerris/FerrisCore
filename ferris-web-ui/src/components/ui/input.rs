use leptos::html;
use leptos::prelude::*;
use strum::AsRefStr;
use tw_merge::tw_merge;

#[derive(Default, Clone, Copy, PartialEq, Eq, AsRefStr)]
#[strum(serialize_all = "lowercase")]
pub enum InputType {
    #[default]
    Text,
    Email,
    Password,
    Number,
    Tel,
    Url,
    Search,
    Date,
    Time,
    #[strum(serialize = "datetime-local")]
    DatetimeLocal,
    Month,
    Week,
    Color,
    File,
    Hidden,
}

#[component]
pub fn Input(
    // Styling
    #[prop(into, optional)] class: String,

    // Common HTML attributes
    #[prop(default = InputType::default())] r#type: InputType,
    #[prop(into, optional)] placeholder: Option<String>,
    #[prop(into, optional)] name: Option<String>,
    #[prop(into, optional)] id: Option<String>,
    #[prop(into, optional)] title: Option<String>,
    #[prop(into, optional)] aria_label: Option<String>,
    #[prop(into, optional)] autocomplete: Option<String>,
    #[prop(optional)] disabled: bool,
    #[prop(optional)] readonly: bool,
    #[prop(optional)] required: bool,
    #[prop(optional)] autofocus: bool,
    #[prop(optional)] minlength: Option<u16>,
    /// Reactive invalid state; drives `aria-invalid` and the shared
    /// `.ferris-input[aria-invalid]` styling.
    #[prop(optional, into)]
    aria_invalid: Option<Signal<bool>>,
    /// Id of the element that explains the field (its error/description).
    #[prop(into, optional)]
    aria_describedby: Option<String>,

    // Number input attributes
    #[prop(into, optional)] min: Option<String>,
    #[prop(into, optional)] max: Option<String>,
    #[prop(into, optional)] step: Option<String>,

    // Two-way binding (like bind:value)
    #[prop(into, optional)] bind_value: Option<RwSignal<String>>,

    /// Called with the input's text after every keystroke. Components do not
    /// receive bare `on:input` DOM listeners, so a widget that owns its own
    /// state (the phone field) passes the callback explicitly instead.
    #[prop(optional, into)]
    on_input: Option<Callback<String>>,
    /// Called when the input loses focus.
    #[prop(optional, into)]
    on_blur: Option<Callback<()>>,

    // Ref for direct DOM access
    #[prop(optional)] node_ref: NodeRef<html::Input>,
) -> impl IntoView {
    let merged_class = tw_merge!(
        "ferris-input flex h-9 max-md:h-10 w-full min-w-0 rounded-md border px-3 py-1 text-base shadow-xs transition-[color,box-shadow] outline-none file:inline-flex file:h-7 file:border-0 file:bg-transparent file:text-sm file:font-medium disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm",
        class
    );

    let type_str = r#type.as_ref();

    match bind_value {
        Some(signal) => view! {
            <input
                data-name="Input"
                type=type_str
                class=merged_class
                placeholder=placeholder
                name=name
                id=id
                title=title
                aria-label=aria_label
                autocomplete=autocomplete
                disabled=disabled
                readonly=readonly
                required=required
                autofocus=autofocus
                minlength=minlength
                min=min
                max=max
                step=step
                aria-invalid=move || {
                    aria_invalid
                        .map(|state| state.get().to_string())
                        .unwrap_or_else(|| "false".to_string())
                }
                aria-describedby=aria_describedby
                bind:value=signal
                node_ref=node_ref
                on:input=move |ev| {
                    if let Some(callback) = on_input {
                        callback.run(event_target_value(&ev));
                    }
                }
                on:blur=move |_| {
                    if let Some(callback) = on_blur {
                        callback.run(());
                    }
                }
            />
        }
        .into_any(),
        None => view! {
            <input
                data-name="Input"
                type=type_str
                class=merged_class
                placeholder=placeholder
                name=name
                id=id
                title=title
                aria-label=aria_label
                autocomplete=autocomplete
                disabled=disabled
                readonly=readonly
                required=required
                autofocus=autofocus
                minlength=minlength
                min=min
                max=max
                step=step
                aria-invalid=move || {
                    aria_invalid
                        .map(|state| state.get().to_string())
                        .unwrap_or_else(|| "false".to_string())
                }
                aria-describedby=aria_describedby
                node_ref=node_ref
                on:input=move |ev| {
                    if let Some(callback) = on_input {
                        callback.run(event_target_value(&ev));
                    }
                }
                on:blur=move |_| {
                    if let Some(callback) = on_blur {
                        callback.run(());
                    }
                }
            />
        }
        .into_any(),
    }
}
