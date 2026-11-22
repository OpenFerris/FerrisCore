use leptos::prelude::*;
use leptos_ui::{clx, void};

#[cfg(target_arch = "wasm32")]
use crate::components::hooks::use_card_carousel;

mod components {
    use super::*;
    // A fixed 320px card would overflow a 320px viewport; `max-w-full` lets it
    // shrink to its container without changing the desktop square.
    clx! {CardCarousel, div, "group rounded-[20px] overflow-hidden relative w-[320px] max-w-full h-[320px] ferris-card-carousel"}
    clx! {CardCarouselOverlay, div, "pb-4 absolute bottom-0 flex flex-col justify-between items-center z-10 h-[calc(50%+32px)] w-full"}
    clx! {CardCarouselNav, div, "opacity-0 invisible group-hover:visible group-hover:opacity-100 transition-opacity duration-[240ms] p-3 flex justify-between items-center w-full"}
    clx! {CardCarouselNavButton, button, "border-0 rounded-full cursor-pointer flex items-center justify-center size-8 [&_svg:not([class*='size-'])]:size-3 bg-accent transition-all duration-[160ms] ease-in-out hover:shadow-sm hover:scale-110 aria-[disabled]:invisible"}
    clx! {CardCarouselIndicators, div, "gap-1 flex"}
    void! {CardCarouselIndicator, span, "rounded-full size-[6px] ferris-card-carousel__indicator opacity-60 aria-[current]:opacity-100"}
    clx! {CardCarouselSlide, div, "snap-center shrink-0 w-full h-full"}
    void! {CardCarouselImage, img, "object-cover w-full h-full"}
}

pub use components::*;

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn CardCarouselTrack(children: Children) -> impl IntoView {
    #[cfg(target_arch = "wasm32")]
    use_card_carousel::init();

    view! {
        <div
            data-name="CardCarouselTrack"
            class="flex overflow-x-scroll w-full h-full snap-x snap-mandatory scroll-smooth touch-pan-x [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        >
            {children()}
        </div>
    }
}
