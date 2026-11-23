use icons::{ChevronsUpDown, Search, X};
use leptos::prelude::*;
use leptos_ui::clx;
use tw_merge::*;

use super::command::{
    Command, CommandEmpty, CommandGroup, CommandGroupLabel, CommandInput, CommandItem, CommandList,
};
use super::input::Input;
use super::popover::{Popover, PopoverAlign, PopoverContent, PopoverTrigger};
use crate::components::utils::country::Country;
use crate::components::utils::phone_number::{PhoneFormat, PhoneNumber};

const COMMON_COUNTRIES: &[Country] = &[
    Country::UnitedStatesOfAmerica,
    Country::UnitedKingdom,
    Country::France,
    Country::Germany,
    Country::Canada,
    Country::Australia,
    Country::Spain,
    Country::Italy,
    Country::Japan,
    Country::China,
    Country::India,
    Country::Brazil,
    Country::Mexico,
];

mod components {
    use super::*;
    clx! {InputPhoneWrapper, div, "flex w-full"}
}

pub use components::*;

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
fn CountryItem(country: Country, selected_country: RwSignal<Country>) -> impl IntoView {
    let search_value = format!(
        "{} {} {}",
        country.name(),
        country.alpha2(),
        country.dial_code_formatted(),
    );
    let is_selected = Signal::derive(move || selected_country.get() == country);

    view! {
        <CommandItem
            value=search_value
            selected=is_selected
            reserve_check_space=true
            on_select=Callback::new(move |_| {
                selected_country.set(country);
            })
        >
            <span class="text-base">{country.flag_emoji()}</span>
            <span class="flex-1 truncate">{country.name()}</span>
            <span class="w-12 text-right text-muted-foreground">{country.dial_code_formatted()}</span>
        </CommandItem>
    }
}

#[component]
pub fn InputPhone(
    #[prop(optional, into)] class: String,
    #[prop(into, optional)] id: Option<String>,
    #[prop(optional)] value_signal: Option<RwSignal<PhoneNumber>>,
    #[prop(optional)] country_signal: Option<RwSignal<Country>>,
    #[prop(optional)] disabled: bool,
    #[prop(optional, into)] invalid: MaybeProp<bool>,
    #[prop(optional, into)] on_blur: Option<Callback<()>>,
) -> impl IntoView {
    let internal_value_signal = RwSignal::new(PhoneNumber::default());
    let internal_country_signal = RwSignal::new(Country::UnitedStatesOfAmerica);

    let value = value_signal.unwrap_or(internal_value_signal);
    let selected_country = country_signal.unwrap_or(internal_country_signal);

    let wrapper_class = tw_merge!("flex w-full", class);

    view! {
        <InputPhoneWrapper class=wrapper_class>
            <Popover align=PopoverAlign::Start>
                <PopoverTrigger
                    // The trigger tracks the input's mobile height, so the two
                    // halves of the control stay the same height on a phone.
                    class="gap-1 px-3 w-auto rounded-r-none border-r-0 max-md:h-10"
                    attr:disabled=disabled
                    attr:aria-label=crate::i18n::t("ui.command.select_country")
                >
                    <span class="text-base">{move || selected_country.get().flag_emoji()}</span>
                    <span class="text-xs text-muted-foreground">
                        {move || selected_country.get().dial_code_formatted()}
                    </span>
                    <ChevronsUpDown class="ml-1 opacity-50 size-3" />
                </PopoverTrigger>

                <PopoverContent class="p-0 w-[280px] max-w-[calc(100vw-1rem)]">
                    <Command>
                        <div class="flex gap-2 items-center px-2 border-b">
                            <Search class="size-4 text-muted-foreground shrink-0" />
                            <CommandInput
                                placeholder=crate::i18n::t("ui.command.country_search")
                                aria_label=crate::i18n::t("ui.command.country_search_label")
                            />
                        </div>
                        <CommandList class="min-h-0 max-h-[280px]">
                            <CommandEmpty>{crate::i18n::t("ui.command.no_results")}</CommandEmpty>

                            // Common countries
                            <CommandGroup>
                                {COMMON_COUNTRIES
                                    .iter()
                                    .map(|&country| {
                                        view! { <CountryItem country selected_country /> }
                                    })
                                    .collect_view()}
                            </CommandGroup>

                            // Separator + rest of countries
                            <CommandGroup>
                                <CommandGroupLabel>{crate::i18n::t("ui.command.all_countries")}</CommandGroupLabel>
                                {Country::all()
                                    .iter()
                                    .filter(|c| !COMMON_COUNTRIES.contains(c))
                                    .map(|&country| {
                                        view! { <CountryItem country selected_country /> }
                                    })
                                    .collect_view()}
                            </CommandGroup>
                        </CommandList>
                    </Command>
                </PopoverContent>
            </Popover>

            // Phone number input - displays formatted, stores raw digits
            <div class="relative min-w-0 flex-1">
                <Input
                    class="pr-8 max-md:pr-10 w-full rounded-l-none"
                    attr:id=id
                    attr:r#type="tel"
                    attr:inputmode="numeric"
                    attr:placeholder=move || PhoneFormat::for_country(selected_country.get()).placeholder()
                    attr:disabled=disabled
                    attr:aria-label=crate::i18n::t("ui.input_phone.label")
                    attr:aria-invalid=move || invalid.get().unwrap_or(false)
                    prop:value=move || value.get().format(selected_country.get())
                    // The listener arrives as a prop: a component only gets DOM
                    // events through the props it declares, and a bare
                    // `on:input` here was never attached — typing showed digits
                    // but the stored value stayed empty.
                    on_input=Callback::new(move |raw: String| {
                        let format = PhoneFormat::for_country(selected_country.get());
                        value.set(PhoneNumber::new(&raw, format.max_digits));
                    })
                    on_blur=Callback::new(move |()| {
                        if let Some(cb) = on_blur {
                            cb.run(());
                        }
                    })
                />
                <Show when=move || !value.get().is_empty() && !disabled>
                    // The clear control is a real button with a 36px tap box on
                    // a phone (`p-0.5` around a 16px glyph was an 18px target);
                    // it is keyboard reachable, which `tabindex="-1"` denied.
                    <button
                        type="button"
                        class="absolute end-2 top-1/2 -translate-y-1/2 rounded-sm p-0.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground max-md:end-1 max-md:flex max-md:size-9 max-md:items-center max-md:justify-center max-md:p-0"
                        aria-label=crate::i18n::t("ui.input_phone.clear")
                        on:click=move |_| value.set(PhoneNumber::default())
                    >
                        <X class="size-4" />
                    </button>
                </Show>
            </div>
        </InputPhoneWrapper>
    }
}
