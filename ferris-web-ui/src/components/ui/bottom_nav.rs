use leptos::prelude::*;
use leptos_ui::clx;

// Every interactive cell of the bar is an `<a>`/`<button>` the caller renders.
// Mark the active one with `aria-current="page"`: the label and the icon colour
// their `aria-[current=page]` state from that attribute, so colour is never the
// only signal. The bar itself is sized by `--bottom__nav__height` (with a
// fallback, since a missing custom property would otherwise leave the grid with
// no height at all).
mod components {
    use super::*;
    clx! {BottomNav, nav, "z-50 w-full border-t border-border bg-background pb-[env(safe-area-inset-bottom,0px)]"}
    clx! {BottomNavGrid, div, "grid grid-flow-col auto-cols-fr h-[var(--bottom__nav__height,4rem)] font-medium"}
    // `truncate` + `max-w-full` keep a long label inside its equal-width column
    // at 320px instead of widening the bar and forcing a horizontal scroll.
    clx! {BottomNavLabel, span, "max-w-full truncate text-sm text-muted-foreground group-hover:text-primary group-aria-[current=page]:text-primary"}
    clx! {BottomNavButton, button,
        "inline-flex flex-col justify-center items-center min-w-0 min-h-11 px-5 group [&_svg]:mb-2 [&_svg]:text-muted-foreground hover:[&_svg]:text-primary aria-[current=page]:[&_svg]:text-primary active:scale-[0.98]",
        "touch-manipulation [-webkit-tap-highlight-color:transparent] select-none [-webkit-touch-callout:none]",
        "outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:ring-inset",
        // A narrow phone cannot afford 2.5rem of side padding per cell.
        "max-md:px-2",
        // * SHORTFIX 🚑 iOS Safari: push icons closer to home indicator
        "supports-[-webkit-touch-callout:none]:justify-end supports-[-webkit-touch-callout:none]:pb-0 supports-[-webkit-touch-callout:none]:translate-y-1"
    }
}

pub use components::*;
