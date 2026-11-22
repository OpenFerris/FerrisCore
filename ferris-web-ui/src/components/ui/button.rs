use leptos::prelude::*;
use leptos_ui::variants;

// TODO 💪 Loading state (demo_use_timeout_fn.rs and demo_button.rs)

variants! {
    Button {
        base: "ferris-button inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 shrink-0 [&_svg]:shrink-0 outline-none  w-fit  hover:cursor-pointer active:scale-[0.98] active:opacity-100 touch-manipulation [-webkit-tap-highlight-color:transparent] select-none [-webkit-touch-callout:none]", // Using hover:cursor-pointer as workaround for href_support.
        variants: {
            variant: {
                Default: "ferris-button--default shadow-xs",
                Destructive: "ferris-button--destructive shadow-xs",
                Outline: "ferris-button--outline border shadow-xs",
                Secondary: "ferris-button--secondary shadow-xs",
                Ghost: "ferris-button--ghost",
                Accent: "ferris-button--accent",
                Link: "ferris-button--link underline-offset-4 hover:underline",
                //
                Warning: "ferris-button--warning",
                Success: "ferris-button--success",
                Bordered: "ferris-button--bordered border",
            },
            // Sizes below carry a `max-md:` floor so a finger gets a ~40px
            // target on phones while the desktop metrics stay exactly as they
            // were. A size whose desktop height is already >= 40px (`Lg`) or
            // whose padding is already generous (`Mobile`) needs no floor.
            size: {
                Default: "h-9 px-4 py-2 has-[>svg]:px-3 max-md:h-10 max-md:px-4",
                Sm: "h-8 rounded-md gap-1.5 px-3 has-[>svg]:px-2.5 max-md:h-10 max-md:px-4",
                Lg: "h-10 rounded-md px-6 has-[>svg]:px-4",
                Icon: "size-9 max-md:size-10",
                IconSm: "size-8 rounded-md max-md:size-10",
                IconXs: "size-6 rounded-md max-md:size-10",
                //
                Mobile: "px-6 py-3 rounded-[24px]",
                Badge: "px-2.5 py-0.5 text-xs"
            }
        },
        component: {
            element: button,
            support_href: true,
            support_aria_current: true
        }
    }
}
