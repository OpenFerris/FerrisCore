use icons::X;
use leptos::context::Provider;
use leptos::prelude::*;
use leptos_ui::clx;
use tw_merge::*;

use super::button::ButtonSize;
use super::button::{Button, ButtonVariant};
use crate::components::hooks::focus_trap::FOCUS_TRAP_JS;
use crate::components::hooks::use_unique_id::use_unique_id_for;

mod components {
    use super::*;
    clx! {SheetHeader, div, "flex flex-col gap-0.5 p-4"}
    clx! {SheetTitle, h2, "font-bold text-2xl"}
    clx! {SheetDescription, p, "ferris-sheet__description"}
    clx! {SheetBody, div, "flex flex-col gap-4"}
    clx! {SheetFooter, footer, "mt-auto flex flex-col gap-2 p-4"}
}

pub use components::*;

/* ========================================================== */
/*                     ✨ CONTEXT ✨                          */
/* ========================================================== */

#[derive(Clone)]
pub struct SheetContext {
    pub target_id: String,
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

pub type SheetVariant = ButtonVariant;
pub type SheetSize = ButtonSize;

#[component]
pub fn Sheet(children: Children, #[prop(optional, into)] class: String) -> impl IntoView {
    let sheet_target_id = use_unique_id_for("sheet");
    let ctx = SheetContext {
        target_id: sheet_target_id,
    };

    let merged_class = tw_merge!("", class);

    view! {
        <Provider value=ctx>
            <div data-name="Sheet" class=merged_class>
                {children()}
            </div>
        </Provider>
    }
}

#[component]
pub fn SheetTrigger(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(default = ButtonVariant::Outline)] variant: ButtonVariant,
    #[prop(default = ButtonSize::Default)] size: ButtonSize,
) -> impl IntoView {
    let ctx = expect_context::<SheetContext>();
    let trigger_id = format!("trigger_{}", ctx.target_id);

    view! {
        <Button class=class attr:id=trigger_id attr:data-sheet-trigger=ctx.target_id variant=variant size=size>
            {children()}
        </Button>
    }
}

#[component]
pub fn SheetClose(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(default = ButtonVariant::Outline)] variant: ButtonVariant,
    #[prop(default = ButtonSize::Default)] size: ButtonSize,
) -> impl IntoView {
    let ctx = expect_context::<SheetContext>();

    view! {
        <Button class=class attr:data-sheet-close=ctx.target_id attr:aria-label="Close sheet" variant=variant size=size>
            {children()}
        </Button>
    }
}

#[component]
pub fn SheetContent(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(default = SheetDirection::Right)] direction: SheetDirection,
    #[prop(default = true)] show_close_button: bool,
) -> impl IntoView {
    let ctx = expect_context::<SheetContext>();

    let backdrop_id = format!("{}_backdrop", ctx.target_id);
    let target_id_for_script = ctx.target_id.clone();
    let backdrop_id_for_script = backdrop_id.clone();

    let merged_class = tw_merge!(
        "ferris-sheet__content fixed z-100 shadow-lg p-6 transition-transform duration-300 overflow-y-auto overscroll-y-contain max-w-[calc(100vw-1rem)] max-md:max-h-[90dvh]",
        direction.initial_position(),
        direction.closed_class(),
        class
    );

    let focus_trap_js = FOCUS_TRAP_JS;

    view! {
        <div
            data-name="SheetBackdrop"
            id=backdrop_id
            data-ferris-backdrop=""
            class="ferris-overlay fixed inset-0 transition-opacity duration-200 pointer-events-none z-60 data-[state=closed]:opacity-0 data-[state=open]:opacity-100"
            data-state="closed"
        />

        <div
            data-name="SheetContent"
            class=merged_class
            id=ctx.target_id
            data-direction=direction.to_string()
            data-state="closed"
            role="dialog"
            aria-modal="true"
            tabindex="-1"
            style="pointer-events: none;"
        >
            <button
                type="button"
                class=tw_merge!(
                    "ferris-sheet__close absolute top-4 right-4 p-1 rounded-sm focus:outline-none [&_svg:not([class*='size-'])]:size-4",
                    if show_close_button { "" } else { "hidden" },
                )
                data-sheet-close=ctx.target_id.clone()
                aria-label="Close sheet"
            >
                <span class="hidden">"Close Sheet"</span>
                <X />
            </button>

            {children()}
        </div>

        <script nonce=crate::csp_nonce()>
            {format!(
                r#"
                {focus_trap_js}
                (function() {{
                    const setupSheet = () => {{
                        const sheet = document.querySelector('#{}');
                        const backdrop = document.querySelector('#{}');
                        const trigger = document.querySelector('[data-sheet-trigger="{}"]');

                        if (!sheet || !backdrop || !trigger) {{
                            setTimeout(setupSheet, 50);
                            return;
                        }}

                        if (sheet.hasAttribute('data-initialized')) {{
                            return;
                        }}
                        sheet.setAttribute('data-initialized', 'true');

                        // One enter/leave edge, driven by `data-state`, so the
                        // trap also works when something else opens the sheet.
                        const trap = window.__ferrisFocusTrap;
                        let trapped = false;
                        const syncFocus = () => {{
                            const isOpen = sheet.getAttribute('data-state') === 'open';
                            if (isOpen && !trapped) {{
                                trapped = true;
                                trap.enter(sheet);
                            }} else if (!isOpen && trapped) {{
                                trapped = false;
                                trap.leave(sheet);
                            }}
                        }};
                        new MutationObserver(syncFocus).observe(sheet, {{
                            attributes: true,
                            attributeFilter: ['data-state'],
                        }});

                        const openSheet = () => {{
                            // Lock scrolling
                            if (window.ScrollLock) window.ScrollLock.lock();

                            sheet.setAttribute('data-state', 'open');
                            backdrop.setAttribute('data-state', 'open');
                            sheet.style.pointerEvents = 'auto';
                            backdrop.style.pointerEvents = 'auto';

                            // Add open class for transform
                            const direction = sheet.getAttribute('data-direction');
                            sheet.classList.remove('translate-x-full', '-translate-x-full', 'translate-y-full', '-translate-y-full');
                            sheet.classList.add('translate-x-0', 'translate-y-0');
                        }};

                        const closeSheet = () => {{
                            sheet.setAttribute('data-state', 'closed');
                            backdrop.setAttribute('data-state', 'closed');
                            sheet.style.pointerEvents = 'none';
                            backdrop.style.pointerEvents = 'none';

                            // Add closed class for transform
                            const direction = sheet.getAttribute('data-direction');
                            sheet.classList.remove('translate-x-0', 'translate-y-0');
                            if (direction === 'Right') sheet.classList.add('translate-x-full');
                            else if (direction === 'Left') sheet.classList.add('-translate-x-full');
                            else if (direction === 'Top') sheet.classList.add('-translate-y-full');
                            else if (direction === 'Bottom') sheet.classList.add('translate-y-full');

                            // Unlock scrolling after animation
                            if (window.ScrollLock) window.ScrollLock.unlock(300);
                        }};

                        // Open sheet when trigger is clicked
                        trigger.addEventListener('click', openSheet);

                        // Close buttons
                        const closeButtons = sheet.querySelectorAll('[data-sheet-close]');
                        closeButtons.forEach(btn => {{
                            btn.addEventListener('click', closeSheet);
                        }});

                        // Close on backdrop click
                        backdrop.addEventListener('click', closeSheet);

                        // Handle ESC key to close
                        document.addEventListener('keydown', (e) => {{
                            if (e.key === 'Escape' && sheet.getAttribute('data-state') === 'open') {{
                                e.preventDefault();
                                closeSheet();
                            }}
                        }});
                    }};

                    if (document.readyState === 'loading') {{
                        document.addEventListener('DOMContentLoaded', setupSheet);
                    }} else {{
                        setupSheet();
                    }}
                }})();
                "#,
                target_id_for_script,
                backdrop_id_for_script,
                target_id_for_script,
            )}
        </script>
    }
}

/* ========================================================== */
/*                     ✨ ENUM ✨                             */
/* ========================================================== */

#[derive(Clone, Copy, strum::AsRefStr, strum::Display)]
pub enum SheetDirection {
    Right,
    Left,
    Top,
    Bottom,
}

impl SheetDirection {
    fn closed_class(self) -> &'static str {
        match self {
            SheetDirection::Right => "translate-x-full",
            SheetDirection::Left => "-translate-x-full",
            SheetDirection::Top => "-translate-y-full",
            SheetDirection::Bottom => "translate-y-full",
        }
    }

    fn initial_position(self) -> &'static str {
        match self {
            SheetDirection::Right => "top-0 right-0 h-full w-[400px]",
            SheetDirection::Left => "top-0 left-0 h-full w-[400px]",
            SheetDirection::Top => "top-0 left-0 w-full h-[400px]",
            // A bottom sheet sits on the home-indicator edge, so its padding has
            // to clear the safe area; on a device without an inset the value is
            // the same 1.5rem the base `p-6` already gives.
            SheetDirection::Bottom => {
                "bottom-0 left-0 w-full h-[400px] max-md:pb-[calc(1.5rem+env(safe-area-inset-bottom,0px))]"
            }
        }
    }
}
