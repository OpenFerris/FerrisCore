use leptos::prelude::*;
use leptos_ui::clx;
use tw_merge::*;

use super::separator::{Separator, SeparatorOrientation};

mod components {
    use super::*;
    clx! {ButtonGroupText, span, "bg-muted flex items-center gap-2 rounded-md border px-4 text-sm font-medium shadow-xs [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4"}
}

pub use components::*;

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn ButtonGroup(
    #[prop(into, optional)] orientation: Signal<ButtonGroupOrientation>,
    #[prop(into, optional)] class: String,
    children: Children,
) -> impl IntoView {
    let merged_class = Memo::new(move |_| {
        let orientation = orientation.get();
        let button_group = ButtonGroupClass { orientation };
        button_group.with_class(class.clone())
    });

    view! {
        <div data-name="ButtonGroup" role="group" class=merged_class>
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

/// `max-w-full`: a connected group never widens the page. Its segments keep
/// their own widths, but the group itself can no longer exceed its parent (a
/// 320px phone with two neighbouring controls).
#[derive(TwClass, Default)]
#[tw(
    class = "flex w-fit max-w-full items-stretch [&>*]:focus-visible:z-10 [&>*]:focus-visible:relative [&>[data-slot=select-trigger]:not([class*='w-'])]:w-fit [&>input]:flex-1 has-[select[aria-hidden=true]:last-child]:[&>[data-slot=select-trigger]:last-of-type]:rounded-r-md has-[>[data-slot=button-group]]:gap-2"
)]
pub struct ButtonGroupClass {
    pub orientation: ButtonGroupOrientation,
}

#[derive(TwVariant)]
pub enum ButtonGroupOrientation {
    #[tw(
        default,
        class = "\
            /* ── Top-level items ───────────────────────── */
            [&>*:not(:first-child)]:rounded-l-none \
            [&>*:not(:first-child)]:border-l-0 \
            [&>*:not(:last-child)]:rounded-r-none \
            /* ── Botón directo hijo ─────────────────────── */
            [&>*:not(:first-child)>button]:rounded-l-none \
            [&>*:not(:first-child)>button]:border-l-0 \
            [&>*:not(:last-child)>button]:rounded-r-none \
            /* ── Botón dentro de un wrapper display:contents ── */
            [&>*:not(:first-child)>.contents>button]:rounded-l-none \
            [&>*:not(:first-child)>.contents>button]:border-l-0 \
            [&>*:not(:last-child)>.contents>button]:rounded-r-none"
    )]
    Horizontal,
    #[tw(class = "flex-col \
            [&>*:not(:first-child)]:rounded-t-none \
            [&>*:not(:first-child)]:border-t-0 \
            [&>*:not(:last-child)]:rounded-b-none \
            [&>*:not(:first-child)>button]:rounded-t-none \
            [&>*:not(:first-child)>button]:border-t-0 \
            [&>*:not(:last-child)>button]:rounded-b-none \
            [&>*:not(:first-child)>.contents>button]:rounded-t-none \
            [&>*:not(:first-child)>.contents>button]:border-t-0 \
            [&>*:not(:last-child)>.contents>button]:rounded-b-none")]
    Vertical,
}

#[component]
pub fn ButtonGroupSeparator(
    #[prop(into, optional, default = SeparatorOrientation::Vertical.into())] orientation: Signal<
        SeparatorOrientation,
    >,
    #[prop(into, optional)] class: String,
) -> impl IntoView {
    let merged_class = tw_merge!(
        "relative !m-0 self-stretch data-[orientation=vertical]:h-auto",
        class
    );

    view! { <Separator attr:data-name="ButtonGroupSeparator" orientation=orientation class=merged_class /> }
}
