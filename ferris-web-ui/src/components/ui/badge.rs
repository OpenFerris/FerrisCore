use leptos::prelude::*;
use leptos_ui::variants;

variants! {
    Badge {
        base: "ferris-badge inline-flex items-center font-semibold rounded-md border w-fit",
        variants: {
            variant: {
                Default: "ferris-badge--default shadow",
                Secondary: "ferris-badge--secondary",
                Accent: "ferris-badge--accent",
                Muted: "ferris-badge--muted",
                Destructive: "ferris-badge--destructive shadow",
                Outline: "ferris-badge--outline",
                Success: "ferris-badge--success",
                Warning: "ferris-badge--warning",
                Info: "ferris-badge--info",
            },
            size: {
                Default: "px-2.5 py-0.5 text-xs",
                Sm: "px-1.5 py-0.5 text-[10px]",
                Lg: "px-3 py-1 text-sm",
            }
        },
        component: {
            element: span
        }
    }
}
