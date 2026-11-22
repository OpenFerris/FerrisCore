use leptos::prelude::*;
use leptos_ui::void;

mod components {
    use super::*;
    void! {LiquidPointerIndicator, div, "block overflow-hidden absolute w-12 h-20 bg-transparent border border-border pointer-events-none mt-[calc(anchor-size(height)*-0.5)] rounded-[2rem]"}
}

pub use components::*;

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn ActionBar(children: Children) -> impl IntoView {
    view! {
        <style>
            {"
            /* CSS-only selected state using radio buttons */
            input[type=\"radio\"]:checked + [data-name=\"ActionBarButton\"] {
            background-color: color-mix(in oklab, var(--destructive) 10%, transparent);
            color: var(--destructive);
            anchor-name: --action-bar-selected;
            }
            }
            "}
        </style>

        <div data-name="ActionBar" class="flex items-center p-2 rounded-2xl border shadow-lg border-input bg-card">
            {children()}

            // SVG filter
            <SvgFilter />
        </div>
    }
}

// TODO 🐛. Not working yet
#[component]
pub fn ActionBarButton(children: Children, target: &'static str) -> impl IntoView {
    const CLASS_LABEL: &str = "flex relative justify-center items-center mx-1 bg-transparent border-0 duration-300 cursor-pointer outline-none action__bar__button px-[15px] py-[10px] before:inset-[-0.4rem] rounded-[50px] transition-[background-color,color] ease-[ease] peer-checked:hover:bg-destructive/10 peer-checked:focus:bg-destructive/10 peer-focus-visible:ring-2 peer-focus-visible:ring-ring before:content-[''] before:absolute hover:bg-muted focus:bg-muted";

    view! {
        // `sr-only`, not `hidden`: the radio is the control, so it must stay in
        // the tab order and keep the label's focus ring reachable.
        <input type="radio" id=target name="action" class="peer sr-only" />
        <label data-name="ActionBarButton" r#for=target class=CLASS_LABEL>
            {children()}
        </label>
    }
}

#[component]
fn SvgFilter() -> impl IntoView {
    view! {
        <svg data-name="DisplacementFilterSVG" width="0" height="0">
            <filter
                id="filter"
                color-interpolation-filters="linearRGB"
                filterUnits="objectBoundingBox"
                primitiveUnits="userSpaceOnUse"
            >
                <feDisplacementMap
                    in="SourceGraphic"
                    in2="SourceGraphic"
                    scale="5"
                    xChannelSelector="A"
                    yChannelSelector="A"
                    x="5"
                    y="-5"
                    width="100%"
                    height="100%"
                    result="displacementMap"
                />
            </filter>
        </svg>
    }
}
