use leptos::prelude::*;
use tw_merge::*;

const PING_ANIMATION: &str = "animate-ping";
const RELATIVE: &str = "relative";

#[component]
pub fn Status(
    #[prop(into, optional)] class: String,
    #[prop(into, optional)] variant: StatusIndactorVariant,
    children: Children,
) -> impl IntoView {
    let merged_class = tw_merge!(RELATIVE, class);

    view! {
        <div class=merged_class>
            {children()} <StatusIndactor variant=variant class=PING_ANIMATION /> <StatusIndactor variant=variant />
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[derive(TwClass, Clone, Copy)]
#[tw(class = "absolute top-0 right-0 -mt-1 -mr-1 rounded-full size-4")]
pub struct StatusIndactorClass {
    pub variant: StatusIndactorVariant,
}

#[derive(TwVariant)]
pub enum StatusIndactorVariant {
    #[tw(default, class = "ferris-status-indicator--default")]
    Default,
    #[tw(class = "ferris-status-indicator--active")]
    Active,
    #[tw(class = "ferris-status-indicator--inactive")]
    Inactive,
    #[tw(class = "ferris-status-indicator--normal")]
    Normal,
}

#[component]
pub fn StatusIndactor(
    #[prop(into, optional)] variant: Signal<StatusIndactorVariant>,
    #[prop(into, optional)] class: String,
) -> impl IntoView {
    let merged_class = move || {
        let status_indicator = StatusIndactorClass {
            variant: variant.get(),
        };
        status_indicator.with_class(class.clone())
    };

    view! { <div class=merged_class /> }
}
